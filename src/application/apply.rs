use thiserror::Error;

use super::ports::{LinkApplier, LinkApplyError, LockfileStore, LockfileStoreError};
use crate::domain::link_plan::{ConflictReason, LinkPlan, PlanAction};
use crate::domain::lockfile::Lockfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOptions {
    pub force: bool,
}

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("source is missing for skill '{skill}' and agent '{agent}'")]
    SourceMissing { skill: String, agent: String },
    #[error("target conflict for skill '{skill}' and agent '{agent}': {reason}")]
    Conflict {
        skill: String,
        agent: String,
        reason: ConflictReason,
    },
    #[error(
        "target symlink drift for skill '{skill}' and agent '{agent}': points to {actual_source}"
    )]
    DriftedSymlink {
        skill: String,
        agent: String,
        actual_source: String,
    },
    #[error(transparent)]
    LinkApply(#[from] LinkApplyError),
    #[error(transparent)]
    LockfileStore(#[from] LockfileStoreError),
}

pub fn apply_link_plan(
    plan: &LinkPlan,
    lockfile: &Lockfile,
    applier: &impl LinkApplier,
    lockfile_store: &impl LockfileStore,
    _options: ApplyOptions,
) -> Result<(), ApplyError> {
    validate_plan_is_safe_to_apply(plan)?;

    for item in &plan.items {
        if item.action == PlanAction::CreateSymlink {
            applier.create_symlink(&item.source, &item.target)?;
        }
    }

    lockfile_store.write(lockfile)?;
    Ok(())
}

fn validate_plan_is_safe_to_apply(plan: &LinkPlan) -> Result<(), ApplyError> {
    for item in &plan.items {
        match &item.action {
            PlanAction::CreateSymlink | PlanAction::AlreadySynced => {}
            PlanAction::SourceMissing => {
                return Err(ApplyError::SourceMissing {
                    skill: item.skill.as_str().to_owned(),
                    agent: item.agent.as_str().to_owned(),
                });
            }
            PlanAction::Conflict { reason } => {
                return Err(ApplyError::Conflict {
                    skill: item.skill.as_str().to_owned(),
                    agent: item.agent.as_str().to_owned(),
                    reason: *reason,
                });
            }
            PlanAction::DriftedSymlink { actual_source } => {
                return Err(ApplyError::DriftedSymlink {
                    skill: item.skill.as_str().to_owned(),
                    agent: item.agent.as_str().to_owned(),
                    actual_source: actual_source.display().to_string(),
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply_link_plan, ApplyError, ApplyOptions};
    use crate::application::ports::{
        LinkApplier, LinkApplyError, LockfileStore, LockfileStoreError,
    };
    use crate::domain::agent::AgentKind;
    use crate::domain::link_plan::{ConflictReason, LinkPlan, LinkPlanItem, PlanAction};
    use crate::domain::lockfile::Lockfile;
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::target::TargetPath;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[derive(Default)]
    struct FakeApplier {
        created: RefCell<Vec<(PathBuf, PathBuf)>>,
    }

    impl LinkApplier for FakeApplier {
        fn create_symlink(
            &self,
            source: &SourcePath,
            target: &TargetPath,
        ) -> Result<(), LinkApplyError> {
            self.created.borrow_mut().push((
                source.as_path().to_path_buf(),
                target.as_path().to_path_buf(),
            ));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeLockfileStore {
        written: Cell<bool>,
    }

    impl LockfileStore for FakeLockfileStore {
        fn write(&self, _lockfile: &Lockfile) -> Result<(), LockfileStoreError> {
            self.written.set(true);
            Ok(())
        }
    }

    fn item(action: PlanAction) -> LinkPlanItem {
        LinkPlanItem {
            skill: SkillName::new("review").unwrap(),
            agent: AgentKind::Pi,
            source: SourcePath::new("skills/review").unwrap(),
            target: TargetPath::new("targets/review").unwrap(),
            action,
        }
    }

    fn lockfile() -> Lockfile {
        Lockfile {
            generated_by: "sksync@test".to_owned(),
            generated_at: "test".to_owned(),
            root: PathBuf::from("."),
            skills: BTreeMap::new(),
        }
    }

    #[test]
    fn create_action_creates_symlink_and_writes_lockfile() {
        let plan = LinkPlan::new(vec![item(PlanAction::CreateSymlink)]);
        let applier = FakeApplier::default();
        let lockfiles = FakeLockfileStore::default();

        apply_link_plan(
            &plan,
            &lockfile(),
            &applier,
            &lockfiles,
            ApplyOptions { force: false },
        )
        .expect("apply succeeds");

        assert_eq!(applier.created.borrow().len(), 1);
        assert!(lockfiles.written.get());
    }

    #[test]
    fn regular_file_conflict_fails_before_apply() {
        let plan = LinkPlan::new(vec![item(PlanAction::Conflict {
            reason: ConflictReason::RegularFile,
        })]);
        let applier = FakeApplier::default();
        let lockfiles = FakeLockfileStore::default();

        let error = apply_link_plan(
            &plan,
            &lockfile(),
            &applier,
            &lockfiles,
            ApplyOptions { force: false },
        )
        .expect_err("conflict fails");

        assert!(matches!(error, ApplyError::Conflict { .. }));
        assert!(applier.created.borrow().is_empty());
        assert!(!lockfiles.written.get());
    }

    #[test]
    fn unexpected_symlink_fails_without_force() {
        let plan = LinkPlan::new(vec![item(PlanAction::DriftedSymlink {
            actual_source: PathBuf::from("other"),
        })]);
        let applier = FakeApplier::default();
        let lockfiles = FakeLockfileStore::default();

        let error = apply_link_plan(
            &plan,
            &lockfile(),
            &applier,
            &lockfiles,
            ApplyOptions { force: false },
        )
        .expect_err("drift fails");

        assert!(matches!(error, ApplyError::DriftedSymlink { .. }));
        assert!(applier.created.borrow().is_empty());
        assert!(!lockfiles.written.get());
    }
}
