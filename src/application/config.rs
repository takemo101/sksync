use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

use thiserror::Error;

use crate::domain::agent::{AgentKind, AgentKindError};
use crate::domain::scope::{Scope, ScopeError};
use crate::domain::skill::{SkillName, SkillNameError, SourcePath, SourcePathError};
use crate::domain::source::InstallSource;

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
    pub default_agents: Vec<AgentKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgent {
    pub kind: AgentKind,
    pub enabled: bool,
    pub scope: Scope,
    pub target_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTargetDir {
    pub target_dir: PathBuf,
    pub scope: Scope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSkill {
    pub name: SkillName,
    pub source: SourcePath,
    pub install_source: Option<InstallSource>,
    pub agents: Vec<AgentKind>,
}

pub fn apply_agent_target_dirs(
    config: &mut ResolvedConfig,
    target_dirs: BTreeMap<String, PathBuf>,
) -> Result<(), ConfigResolveError> {
    apply_agent_target_mappings(
        config,
        target_dirs
            .into_iter()
            .map(|(name, target_dir)| {
                (
                    name,
                    AgentTargetDir {
                        target_dir,
                        scope: Scope::User,
                    },
                )
            })
            .collect(),
    )
}

pub fn apply_agent_target_mappings(
    config: &mut ResolvedConfig,
    target_dirs: BTreeMap<String, AgentTargetDir>,
) -> Result<(), ConfigResolveError> {
    for (name, mapping) in target_dirs {
        let kind =
            AgentKind::from_str(&name).map_err(|source| ConfigResolveError::InvalidAgentName {
                name: name.clone(),
                source,
            })?;
        let key = kind.as_str().to_owned();
        config
            .agents
            .entry(key)
            .and_modify(|agent| {
                if agent.target_dir.is_none() {
                    agent.target_dir = Some(mapping.target_dir.clone());
                    agent.scope = mapping.scope;
                }
            })
            .or_insert(ResolvedAgent {
                kind,
                enabled: true,
                scope: mapping.scope,
                target_dir: Some(mapping.target_dir),
            });
    }
    Ok(())
}
