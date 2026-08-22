use thiserror::Error;

use super::config::ResolvedConfig;
use super::ports::{
    LinkStore, LinkStoreError, SourceStore, SourceStoreError, TargetResolver, TargetResolverError,
    TargetState,
};
use crate::domain::link_plan::{ConflictReason, LinkOwner, LinkPlan, LinkPlanItem, PlanAction};
use crate::domain::skill::SourcePath;
use crate::domain::target::{TargetPath, TargetPathError};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Error)]
pub enum PlanError {
    #[error(transparent)]
    SourceStore(#[from] SourceStoreError),
    #[error(transparent)]
    LinkStore(#[from] LinkStoreError),
    #[error(transparent)]
    TargetResolver(#[from] TargetResolverError),
    #[error("target path is invalid: {0}")]
    InvalidTarget(#[from] TargetPathError),
    #[error("skill '{skill}' references missing agent '{agent}'")]
    MissingAgent { skill: String, agent: String },
    #[error("target '{target}' resolves to multiple desired sources: {details}")]
    TargetSourceConflict { target: String, details: String },
}

struct DesiredGroup {
    source: SourcePath,
    target: TargetPath,
    owners: Vec<LinkOwner>,
}

pub fn build_link_plan(
    config: &ResolvedConfig,
    source_store: &impl SourceStore,
    link_store: &impl LinkStore,
    target_resolver: &impl TargetResolver,
) -> Result<LinkPlan, PlanError> {
    let groups = build_desired_groups(config, target_resolver)?;
    let mut items = Vec::with_capacity(groups.len());

    for group in groups {
        let source_exists = source_store.source_exists(&group.source)?;
        let action = if source_exists {
            inspect_action(link_store, &group.target, &group.source)?
        } else {
            PlanAction::SourceMissing
        };
        items.push(LinkPlanItem {
            owners: group.owners,
            source: group.source,
            target: group.target,
            action,
        });
    }

    Ok(LinkPlan::new(items))
}

pub fn build_desired_link_plan(
    config: &ResolvedConfig,
    target_resolver: &impl TargetResolver,
) -> Result<LinkPlan, PlanError> {
    Ok(LinkPlan::new(
        build_desired_groups(config, target_resolver)?
            .into_iter()
            .map(|group| LinkPlanItem {
                owners: group.owners,
                source: group.source,
                target: group.target,
                action: PlanAction::CreateSymlink,
            })
            .collect(),
    ))
}

fn build_desired_groups(
    config: &ResolvedConfig,
    target_resolver: &impl TargetResolver,
) -> Result<Vec<DesiredGroup>, PlanError> {
    let mut groups = BTreeMap::<PathBuf, DesiredGroup>::new();

    for skill in &config.skills {
        for agent in &skill.agents {
            let agent_config =
                config
                    .agents
                    .get(agent.as_str())
                    .ok_or_else(|| PlanError::MissingAgent {
                        skill: skill.name.as_str().to_owned(),
                        agent: agent.as_str().to_owned(),
                    })?;
            if !agent_config.enabled {
                continue;
            }

            let target_dir = target_resolver.resolve_agent_target(
                agent,
                agent_config.scope,
                agent_config.target_dir.as_deref(),
            )?;
            let target = TargetPath::new(target_dir.as_path().join(skill.name.as_str()))?;
            let key = target.as_path().to_path_buf();
            let owner = LinkOwner {
                skill: skill.name.clone(),
                agent: agent.clone(),
            };

            if let Some(group) = groups.get_mut(&key) {
                if group.source != skill.source {
                    let current_agents = group
                        .owners
                        .iter()
                        .map(|owner| owner.agent.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(PlanError::TargetSourceConflict {
                        target: target.as_path().display().to_string(),
                        details: format!(
                            "{} ({current_agents}) vs {} ({})",
                            group.source.as_path().display(),
                            skill.source.as_path().display(),
                            agent.as_str(),
                        ),
                    });
                }
                group.owners.push(owner);
            } else {
                groups.insert(
                    key,
                    DesiredGroup {
                        source: skill.source.clone(),
                        target,
                        owners: vec![owner],
                    },
                );
            }
        }
    }

    let mut groups = groups.into_values().collect::<Vec<_>>();
    for group in &mut groups {
        group.owners.sort_by(|left, right| {
            (left.skill.as_str(), left.agent.as_str())
                .cmp(&(right.skill.as_str(), right.agent.as_str()))
        });
    }
    Ok(groups)
}

fn inspect_action(
    link_store: &impl LinkStore,
    target: &TargetPath,
    source: &crate::domain::skill::SourcePath,
) -> Result<PlanAction, LinkStoreError> {
    Ok(match link_store.inspect_target(target, source)? {
        TargetState::Missing => PlanAction::CreateSymlink,
        TargetState::SymlinkToExpectedSource => PlanAction::AlreadySynced,
        TargetState::SymlinkToUnexpectedSource { actual_source } => {
            PlanAction::DriftedSymlink { actual_source }
        }
        TargetState::RegularFileConflict => PlanAction::Conflict {
            reason: ConflictReason::RegularFile,
        },
        TargetState::DirectoryConflict => PlanAction::Conflict {
            reason: ConflictReason::Directory,
        },
        TargetState::BrokenSymlink { .. } => PlanAction::Conflict {
            reason: ConflictReason::BrokenSymlink,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{build_desired_link_plan, build_link_plan, PlanError};
    use crate::application::config::{ResolvedAgent, ResolvedConfig, ResolvedSkill};
    use crate::application::ports::{
        LinkStore, LinkStoreError, SourceStore, SourceStoreError, TargetResolver,
        TargetResolverError, TargetState,
    };
    use crate::domain::agent::AgentKind;
    use crate::domain::link_plan::{ConflictReason, PlanAction};
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::target::TargetPath;
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    struct FakeSourceStore {
        exists: bool,
    }

    impl SourceStore for FakeSourceStore {
        fn source_exists(&self, _source: &SourcePath) -> Result<bool, SourceStoreError> {
            Ok(self.exists)
        }
    }

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

    struct CountingLinkStore {
        state: TargetState,
        inspections: Cell<usize>,
    }

    impl CountingLinkStore {
        fn new(state: TargetState) -> Self {
            Self {
                state,
                inspections: Cell::new(0),
            }
        }
    }

    impl LinkStore for CountingLinkStore {
        fn inspect_target(
            &self,
            _target: &TargetPath,
            _expected_source: &SourcePath,
        ) -> Result<TargetState, LinkStoreError> {
            self.inspections.set(self.inspections.get() + 1);
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
                scope: Scope::User,
                message: error.to_string(),
            })
        }
    }

    struct SharedTargetResolver;

    impl TargetResolver for SharedTargetResolver {
        fn resolve_agent_target(
            &self,
            agent: &AgentKind,
            _scope: Scope,
            _target_dir_override: Option<&Path>,
        ) -> Result<TargetPath, TargetResolverError> {
            TargetPath::new("/targets/shared").map_err(|error| TargetResolverError::Resolve {
                agent: agent.as_str().to_owned(),
                scope: Scope::User,
                message: error.to_string(),
            })
        }
    }

    fn config() -> ResolvedConfig {
        let mut agents = BTreeMap::new();
        agents.insert(
            "pi".to_owned(),
            ResolvedAgent {
                kind: AgentKind::Pi,
                enabled: true,
                scope: Scope::User,
                target_dir: None,
            },
        );

        ResolvedConfig {
            skill_dir: SourcePath::new("skills").unwrap(),
            agents,
            skills: vec![ResolvedSkill {
                name: SkillName::new("review").unwrap(),
                source: SourcePath::new("skills/review").unwrap(),
                install_source: None,
                include: None,
                agents: vec![AgentKind::Pi],
            }],
            default_agents: Vec::new(),
        }
    }

    fn shared_config(source: SourcePath, skill_agents: &[AgentKind]) -> ResolvedConfig {
        let agents = skill_agents
            .iter()
            .cloned()
            .map(|kind| {
                (
                    kind.as_str().to_owned(),
                    ResolvedAgent {
                        kind,
                        enabled: true,
                        scope: Scope::User,
                        target_dir: None,
                    },
                )
            })
            .collect();

        ResolvedConfig {
            skill_dir: SourcePath::new("skills").unwrap(),
            agents,
            skills: vec![ResolvedSkill {
                name: SkillName::new("review").unwrap(),
                source,
                install_source: None,
                include: None,
                agents: skill_agents.to_vec(),
            }],
            default_agents: Vec::new(),
        }
    }

    fn plan_action_for(state: TargetState) -> PlanAction {
        let plan = build_link_plan(
            &config(),
            &FakeSourceStore { exists: true },
            &FakeLinkStore { state },
            &FakeTargetResolver,
        )
        .expect("plan builds");

        plan.items[0].action.clone()
    }

    #[test]
    fn missing_target_becomes_create_symlink_action() {
        assert_eq!(
            plan_action_for(TargetState::Missing),
            PlanAction::CreateSymlink
        );
    }

    #[test]
    fn synced_target_becomes_already_synced_action() {
        assert_eq!(
            plan_action_for(TargetState::SymlinkToExpectedSource),
            PlanAction::AlreadySynced
        );
    }

    #[test]
    fn regular_file_becomes_conflict_action() {
        assert_eq!(
            plan_action_for(TargetState::RegularFileConflict),
            PlanAction::Conflict {
                reason: ConflictReason::RegularFile,
            }
        );
    }

    #[test]
    fn unexpected_symlink_becomes_drifted_action() {
        assert_eq!(
            plan_action_for(TargetState::SymlinkToUnexpectedSource {
                actual_source: PathBuf::from("/other/source"),
            }),
            PlanAction::DriftedSymlink {
                actual_source: PathBuf::from("/other/source"),
            }
        );
    }

    #[test]
    fn missing_source_becomes_source_missing_action() {
        let plan = build_link_plan(
            &config(),
            &FakeSourceStore { exists: false },
            &FakeLinkStore {
                state: TargetState::Missing,
            },
            &FakeTargetResolver,
        )
        .expect("plan builds");

        assert_eq!(plan.items[0].action, PlanAction::SourceMissing);
    }

    #[test]
    fn shared_agents_produce_one_physical_plan_item_and_one_inspection() {
        let universal = AgentKind::custom("universal").unwrap();
        let config = shared_config(
            SourcePath::new("skills/review").unwrap(),
            &[AgentKind::Pi, universal],
        );
        let links = CountingLinkStore::new(TargetState::Missing);

        let plan = build_link_plan(
            &config,
            &FakeSourceStore { exists: true },
            &links,
            &SharedTargetResolver,
        )
        .expect("plan builds");

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].owners.len(), 2);
        assert_eq!(links.inspections.get(), 1);
        assert_eq!(plan.items[0].action, PlanAction::CreateSymlink);
    }

    #[test]
    fn desired_plan_groups_shared_agents() {
        let universal = AgentKind::custom("universal").unwrap();
        let config = shared_config(
            SourcePath::new("skills/review").unwrap(),
            &[AgentKind::Pi, universal],
        );

        let plan = build_desired_link_plan(&config, &SharedTargetResolver).expect("plan builds");

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].owners.len(), 2);
    }

    #[test]
    fn same_target_with_different_sources_is_rejected_before_inspection() {
        let universal = AgentKind::custom("universal").unwrap();
        let mut config = shared_config(
            SourcePath::new("skills/review-a").unwrap(),
            &[AgentKind::Pi, universal.clone()],
        );
        config.skills[0].agents = vec![AgentKind::Pi];
        config.skills.push(ResolvedSkill {
            name: SkillName::new("review").unwrap(),
            source: SourcePath::new("skills/review-b").unwrap(),
            install_source: None,
            include: None,
            agents: vec![universal],
        });
        let links = CountingLinkStore::new(TargetState::Missing);

        let error = build_link_plan(
            &config,
            &FakeSourceStore { exists: true },
            &links,
            &SharedTargetResolver,
        )
        .expect_err("desired source conflict blocks planning");

        assert!(matches!(error, PlanError::TargetSourceConflict { .. }));
        assert_eq!(links.inspections.get(), 0);
    }

    #[test]
    fn target_path_includes_skill_name() {
        let plan = build_link_plan(
            &config(),
            &FakeSourceStore { exists: true },
            &FakeLinkStore {
                state: TargetState::Missing,
            },
            &FakeTargetResolver,
        )
        .expect("plan builds");

        assert_eq!(
            plan.items[0].target.as_path(),
            Path::new("/targets/pi/review")
        );
    }
}
