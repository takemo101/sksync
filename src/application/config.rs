use std::collections::BTreeMap;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSkill {
    pub name: SkillName,
    pub source: SourcePath,
    pub agents: Vec<AgentKind>,
}
