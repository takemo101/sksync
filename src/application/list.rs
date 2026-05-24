use crate::application::config::ResolvedConfig;
use crate::application::ports::{LinkStore, TargetResolver, TargetState};
use crate::domain::lockfile::Lockfile;
use crate::domain::target::TargetPath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListReport {
    pub skills: Vec<ListedSkill>,
}

impl ListReport {
    pub fn display_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for skill in &self.skills {
            lines.push(match &skill.locked_hash {
                Some(hash) => format!("skill: {} (locked {hash})", skill.name),
                None => format!("skill: {}", skill.name),
            });
            for target in &skill.targets {
                lines.push(format!(
                    "  {} -> {} [{}]",
                    target.agent,
                    target.target.display(),
                    target.state
                ));
            }
        }

        if lines.is_empty() {
            lines.push("No skills configured.".to_owned());
        }

        lines
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedSkill {
    pub name: String,
    pub locked_hash: Option<String>,
    pub targets: Vec<ListedTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedTarget {
    pub agent: String,
    pub target: std::path::PathBuf,
    pub state: ListedTargetState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListedTargetState {
    Missing,
    Synced,
    Drifted,
    Conflict,
    BrokenSymlink,
    SourceMissing,
    InspectFailed(String),
    ResolveFailed(String),
}

impl std::fmt::Display for ListedTargetState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Missing => "missing".to_owned(),
            Self::Synced => "synced".to_owned(),
            Self::Drifted => "drifted".to_owned(),
            Self::Conflict => "conflict".to_owned(),
            Self::BrokenSymlink => "broken".to_owned(),
            Self::SourceMissing => "source-missing".to_owned(),
            Self::InspectFailed(message) => format!("inspect-failed: {message}"),
            Self::ResolveFailed(message) => format!("resolve-failed: {message}"),
        };
        formatter.write_str(&value)
    }
}

pub fn list_skills(
    config: &ResolvedConfig,
    lockfile: Option<&Lockfile>,
    link_store: &impl LinkStore,
    target_resolver: &impl TargetResolver,
) -> ListReport {
    let mut skills = Vec::new();

    for skill in &config.skills {
        let locked_hash = lockfile
            .and_then(|lockfile| lockfile.skills.get(&skill.name))
            .map(|locked| locked.hash.as_str().to_owned());
        let source_exists = skill.source.as_path().exists();
        let mut targets = Vec::new();

        for agent in &skill.agents {
            let Some(agent_config) = config.agents.get(agent.as_str()) else {
                continue;
            };
            if !agent_config.enabled {
                continue;
            }

            let target = match target_resolver.resolve_agent_target(
                agent,
                agent_config.scope,
                agent_config.target_dir.as_deref(),
            ) {
                Ok(target_dir) => TargetPath::new(target_dir.as_path().join(skill.name.as_str())),
                Err(error) => {
                    targets.push(ListedTarget {
                        agent: agent.as_str().to_owned(),
                        target: std::path::PathBuf::new(),
                        state: ListedTargetState::ResolveFailed(error.to_string()),
                    });
                    continue;
                }
            };
            let target = match target {
                Ok(target) => target,
                Err(error) => {
                    targets.push(ListedTarget {
                        agent: agent.as_str().to_owned(),
                        target: std::path::PathBuf::new(),
                        state: ListedTargetState::ResolveFailed(error.to_string()),
                    });
                    continue;
                }
            };

            let state = if source_exists {
                match link_store.inspect_target(&target, &skill.source) {
                    Ok(TargetState::Missing) => ListedTargetState::Missing,
                    Ok(TargetState::SymlinkToExpectedSource) => ListedTargetState::Synced,
                    Ok(TargetState::SymlinkToUnexpectedSource { .. }) => ListedTargetState::Drifted,
                    Ok(TargetState::RegularFileConflict | TargetState::DirectoryConflict) => {
                        ListedTargetState::Conflict
                    }
                    Ok(TargetState::BrokenSymlink { .. }) => ListedTargetState::BrokenSymlink,
                    Err(error) => ListedTargetState::InspectFailed(error.to_string()),
                }
            } else {
                ListedTargetState::SourceMissing
            };

            targets.push(ListedTarget {
                agent: agent.as_str().to_owned(),
                target: target.as_path().to_path_buf(),
                state,
            });
        }

        skills.push(ListedSkill {
            name: skill.name.as_str().to_owned(),
            locked_hash,
            targets,
        });
    }

    ListReport { skills }
}

#[cfg(test)]
mod tests {
    use super::{list_skills, ListedTargetState};
    use crate::application::config::{ResolvedAgent, ResolvedConfig, ResolvedSkill};
    use crate::application::ports::{
        LinkStore, LinkStoreError, TargetResolver, TargetResolverError, TargetState,
    };
    use crate::domain::agent::AgentKind;
    use crate::domain::lockfile::{Digest, LockedSkill, Lockfile};
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::target::TargetPath;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    struct FakeLinkStore {
        state: TargetState,
    }

    impl LinkStore for FakeLinkStore {
        fn inspect_target(
            &self,
            _target: &TargetPath,
            _expected_source: &SourcePath,
        ) -> Result<TargetState, LinkStoreError> {
            Ok(self.state.clone())
        }
    }

    struct FakeTargetResolver;

    impl TargetResolver for FakeTargetResolver {
        fn resolve_agent_target(
            &self,
            _agent: &AgentKind,
            _scope: Scope,
            _target_dir_override: Option<&Path>,
        ) -> Result<TargetPath, TargetResolverError> {
            TargetPath::new("/targets/pi").map_err(|error| TargetResolverError::Resolve {
                agent: "pi".to_owned(),
                scope: Scope::Project,
                message: error.to_string(),
            })
        }
    }

    fn config(source: SourcePath) -> ResolvedConfig {
        let mut agents = BTreeMap::new();
        agents.insert(
            "pi".to_owned(),
            ResolvedAgent {
                kind: AgentKind::Pi,
                enabled: true,
                scope: Scope::Project,
                target_dir: None,
            },
        );
        ResolvedConfig {
            skill_dir: SourcePath::new("skills").unwrap(),
            agents,
            skills: vec![ResolvedSkill {
                name: SkillName::new("review").unwrap(),
                source,
                install_source: None,
                agents: vec![AgentKind::Pi],
            }],
            default_agents: Vec::new(),
        }
    }

    #[test]
    fn displays_skill_name_and_target_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source = temp_dir.path().join("review");
        std::fs::create_dir(&source).unwrap();
        let report = list_skills(
            &config(SourcePath::new(source).unwrap()),
            None,
            &FakeLinkStore {
                state: TargetState::Missing,
            },
            &FakeTargetResolver,
        );

        assert_eq!(report.skills[0].name, "review");
        assert_eq!(
            report.skills[0].targets[0].target,
            PathBuf::from("/targets/pi/review")
        );
        assert_eq!(
            report.skills[0].targets[0].state,
            ListedTargetState::Missing
        );
    }

    #[test]
    fn displays_synced_and_conflict_states() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source = temp_dir.path().join("review");
        std::fs::create_dir(&source).unwrap();
        let synced = list_skills(
            &config(SourcePath::new(&source).unwrap()),
            None,
            &FakeLinkStore {
                state: TargetState::SymlinkToExpectedSource,
            },
            &FakeTargetResolver,
        );
        let conflict = list_skills(
            &config(SourcePath::new(source).unwrap()),
            None,
            &FakeLinkStore {
                state: TargetState::RegularFileConflict,
            },
            &FakeTargetResolver,
        );

        assert_eq!(synced.skills[0].targets[0].state, ListedTargetState::Synced);
        assert_eq!(
            conflict.skills[0].targets[0].state,
            ListedTargetState::Conflict
        );
    }

    #[test]
    fn displays_locked_hash_when_lockfile_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source = temp_dir.path().join("review");
        std::fs::create_dir(&source).unwrap();
        let mut skills = BTreeMap::new();
        skills.insert(
            SkillName::new("review").unwrap(),
            LockedSkill {
                source: SourcePath::new(&source).unwrap(),
                install_source: None,
                hash: Digest::new("sha256-locked").unwrap(),
                files: Vec::new(),
                targets: Vec::new(),
            },
        );
        let lockfile = Lockfile {
            generated_by: "test".to_owned(),
            generated_at: "test".to_owned(),
            root: PathBuf::from("."),
            skills,
        };

        let report = list_skills(
            &config(SourcePath::new(source).unwrap()),
            Some(&lockfile),
            &FakeLinkStore {
                state: TargetState::Missing,
            },
            &FakeTargetResolver,
        );

        assert_eq!(
            report.skills[0].locked_hash.as_deref(),
            Some("sha256-locked")
        );
    }
}
