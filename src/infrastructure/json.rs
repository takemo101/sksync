use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Deserialize;

use crate::application::config::{
    ConfigResolveError, ResolvedAgent, ResolvedConfig, ResolvedSkill,
};
use crate::application::ports::{display_path, ConfigStore, ConfigStoreError};
use crate::domain::agent::AgentKind;
use crate::domain::scope::Scope;
use crate::domain::skill::{SkillName, SourcePath};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawConfig {
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    pub skill_dir: PathBuf,
    pub agents: BTreeMap<String, RawAgentConfig>,
    pub skills: BTreeMap<String, RawSkillConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAgentConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub scope: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSkillConfig {
    pub source: Option<PathBuf>,
    #[serde(default)]
    pub agents: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

impl RawConfig {
    pub fn resolve(self) -> Result<ResolvedConfig, ConfigResolveError> {
        let skill_dir = SourcePath::new(self.skill_dir)?;
        let mut agents = BTreeMap::new();
        let mut known_agents = BTreeSet::new();

        for (name, raw_agent) in self.agents {
            let kind = parse_agent_kind(&name)?;
            let key = kind.as_str().to_owned();
            let scope = Scope::from_str(&raw_agent.scope).map_err(|source| {
                ConfigResolveError::InvalidScope {
                    agent: name.clone(),
                    source,
                }
            })?;

            known_agents.insert(key.clone());
            agents.insert(
                key,
                ResolvedAgent {
                    kind,
                    enabled: raw_agent.enabled,
                    scope,
                },
            );
        }

        let mut skills = Vec::with_capacity(self.skills.len());
        for (name, raw_skill) in self.skills {
            let skill_name = SkillName::new(name.clone()).map_err(|source| {
                ConfigResolveError::InvalidSkillName {
                    name: name.clone(),
                    source,
                }
            })?;
            let source_path = raw_skill
                .source
                .unwrap_or_else(|| skill_dir.as_path().join(skill_name.as_str()));
            let source = SourcePath::new(source_path).map_err(|source| {
                ConfigResolveError::InvalidSkillSource {
                    skill: name.clone(),
                    source,
                }
            })?;
            let mut skill_agents = Vec::with_capacity(raw_skill.agents.len());

            for agent in raw_skill.agents {
                let kind = parse_agent_kind(&agent)?;
                if !known_agents.contains(kind.as_str()) {
                    return Err(ConfigResolveError::UnknownAgent { skill: name, agent });
                }
                skill_agents.push(kind);
            }

            skills.push(ResolvedSkill {
                name: skill_name,
                source,
                agents: skill_agents,
            });
        }

        Ok(ResolvedConfig {
            skill_dir,
            agents,
            skills,
        })
    }
}

fn parse_agent_kind(name: &str) -> Result<AgentKind, ConfigResolveError> {
    AgentKind::from_str(name).map_err(|source| ConfigResolveError::InvalidAgentName {
        name: name.to_owned(),
        source,
    })
}

#[derive(Debug, Clone)]
pub struct FileConfigStore {
    path: PathBuf,
}

impl FileConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConfigStore for FileConfigStore {
    fn load(&self) -> Result<ResolvedConfig, ConfigStoreError> {
        let content =
            std::fs::read_to_string(&self.path).map_err(|source| ConfigStoreError::Read {
                path: display_path(&self.path),
                source,
            })?;
        let raw = serde_json::from_str::<RawConfig>(&content).map_err(|source| {
            ConfigStoreError::Parse {
                path: display_path(&self.path),
                source,
            }
        })?;

        raw.resolve().map_err(ConfigStoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::{FileConfigStore, RawConfig};
    use crate::application::config::ConfigResolveError;
    use crate::application::ports::ConfigStore;
    use crate::domain::agent::AgentKind;
    use crate::domain::scope::Scope;
    use std::path::Path;

    #[test]
    fn parses_example_config() {
        let raw =
            serde_json::from_str::<RawConfig>(include_str!("../../sksync.config.example.json"))
                .expect("example config parses");
        let config = raw.resolve().expect("example config resolves");

        assert_eq!(config.skill_dir.as_path(), Path::new("./skills"));
        assert_eq!(config.agents.len(), 5);
        assert_eq!(config.skills.len(), 1);
        assert_eq!(config.skills[0].name.as_str(), "example-skill");
        assert_eq!(config.skills[0].agents.len(), 5);
        assert_eq!(config.agents["pi"].kind, AgentKind::Pi);
        assert_eq!(config.agents["pi"].scope, Scope::User);
    }

    #[test]
    fn rejects_missing_agent_reference() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./skills",
              "agents": { "pi": { "enabled": true, "scope": "user" } },
              "skills": { "review": { "agents": ["missing"] } }
            }"#,
        )
        .expect("raw config parses");

        assert_eq!(
            raw.resolve(),
            Err(ConfigResolveError::UnknownAgent {
                skill: "review".to_owned(),
                agent: "missing".to_owned(),
            })
        );
    }

    #[test]
    fn fills_missing_skill_source_from_skill_dir_and_name() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./skills",
              "agents": { "pi": { "enabled": true, "scope": "project" } },
              "skills": { "review": { "agents": ["pi"] } }
            }"#,
        )
        .expect("raw config parses");
        let config = raw.resolve().expect("config resolves");

        assert_eq!(
            config.skills[0].source.as_path(),
            Path::new("./skills/review")
        );
    }

    #[test]
    fn file_config_store_loads_config_from_disk() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            include_str!("../../sksync.config.example.json"),
        )
        .expect("write config fixture");

        let store = FileConfigStore::new(&config_path);
        let config = store.load().expect("file config loads");

        assert_eq!(store.path(), config_path.as_path());
        assert_eq!(config.skills[0].name.as_str(), "example-skill");
    }
}
