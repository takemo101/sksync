use thiserror::Error;

use super::config::ResolvedConfig;
use super::ports::{
    LinkStore, LinkStoreError, SourceStore, SourceStoreError, TargetResolver, TargetResolverError,
    TargetState,
};
use crate::domain::link_plan::{ConflictReason, LinkPlan, LinkPlanItem, PlanAction};
use crate::domain::target::{TargetPath, TargetPathError};

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
}

pub fn build_link_plan(
    config: &ResolvedConfig,
    source_store: &impl SourceStore,
    link_store: &impl LinkStore,
    target_resolver: &impl TargetResolver,
) -> Result<LinkPlan, PlanError> {
    let mut items = Vec::new();

    for skill in &config.skills {
        let source_exists = source_store.source_exists(&skill.source)?;

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

            let target_dir =
                target_resolver.resolve_agent_target(agent, agent_config.scope, None)?;
            let target = TargetPath::new(target_dir.as_path().join(skill.name.as_str()))?;
            let action = if source_exists {
                inspect_action(link_store, &target, &skill.source)?
            } else {
                PlanAction::SourceMissing
            };

            items.push(LinkPlanItem {
                skill: skill.name.clone(),
                agent: agent.clone(),
                source: skill.source.clone(),
                target,
                action,
            });
        }
    }

    Ok(LinkPlan::new(items))
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
    use super::build_link_plan;
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

    fn config() -> ResolvedConfig {
        let mut agents = BTreeMap::new();
        agents.insert(
            "pi".to_owned(),
            ResolvedAgent {
                kind: AgentKind::Pi,
                enabled: true,
                scope: Scope::User,
            },
        );

        ResolvedConfig {
            skill_dir: SourcePath::new("skills").unwrap(),
            agents,
            skills: vec![ResolvedSkill {
                name: SkillName::new("review").unwrap(),
                source: SourcePath::new("skills/review").unwrap(),
                agents: vec![AgentKind::Pi],
            }],
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
