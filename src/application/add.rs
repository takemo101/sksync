use anyhow::Result;

use crate::application::apply::{apply_link_plan, ApplyOptions};
use crate::application::config::ResolvedConfig;
use crate::application::plan::build_link_plan;
use crate::application::ports::{
    DependencyConfigStore, LinkApplier, LinkStore, LockfileStore, SkillInstaller, SourceStore,
    TargetResolver,
};
use crate::application::update::{apply_update_report_sources, update_dependencies, UpdateReport};
use crate::domain::link_plan::LinkPlan;
use crate::domain::lockfile::Lockfile;

#[derive(Debug, Clone)]
pub struct AddSelection {
    pub skill_name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedDependency {
    pub skill_name: String,
    pub source: String,
}

#[derive(Debug)]
pub struct AddWorkflowReport {
    pub added: Vec<AddedDependency>,
    pub update_report: UpdateReport,
    pub plan: LinkPlan,
}

pub struct AddWorkflow<'a, D, I, F, L, T> {
    pub dependency_store: &'a D,
    pub installer: &'a I,
    pub fs_store: &'a F,
    pub lockfile_store: &'a L,
    pub target_resolver: &'a T,
}

pub fn run_add_workflow<D, I, F, L, T>(
    selections: Vec<AddSelection>,
    agents: &[String],
    load_config: impl FnOnce() -> Result<ResolvedConfig>,
    build_lockfile: impl FnOnce(&ResolvedConfig, &LinkPlan) -> Result<Lockfile>,
    workflow: AddWorkflow<'_, D, I, F, L, T>,
) -> Result<AddWorkflowReport>
where
    D: DependencyConfigStore,
    I: SkillInstaller,
    F: SourceStore + LinkStore + LinkApplier,
    L: LockfileStore,
    T: TargetResolver,
{
    let mut added = Vec::new();
    for selection in selections {
        workflow.dependency_store.add_dependency(
            &selection.skill_name,
            &selection.source,
            agents,
        )?;
        added.push(AddedDependency {
            skill_name: selection.skill_name,
            source: selection.source,
        });
    }

    let mut config = load_config()?;
    let update_report = update_dependencies(&config, workflow.installer)?;
    apply_update_report_sources(&mut config, &update_report);
    let plan = build_link_plan(
        &config,
        workflow.fs_store,
        workflow.fs_store,
        workflow.target_resolver,
    )?;
    let lockfile = build_lockfile(&config, &plan)?;
    apply_link_plan(
        &plan,
        &lockfile,
        workflow.fs_store,
        workflow.lockfile_store,
        ApplyOptions {
            force: false,
            skip_blocked_targets: true,
        },
    )?;

    Ok(AddWorkflowReport {
        added,
        update_report,
        plan,
    })
}

#[cfg(test)]
mod tests {
    use super::{run_add_workflow, AddSelection, AddWorkflow};
    use crate::application::config::{ResolvedAgent, ResolvedConfig, ResolvedSkill};
    use crate::application::ports::{
        DependencyConfigStore, DependencyConfigStoreError, InstalledSkillSource, LinkApplier,
        LinkApplyError, LinkStore, LinkStoreError, LockfileStore, LockfileStoreError,
        SkillInstallError, SkillInstaller, SourceStore, SourceStoreError, TargetResolver,
        TargetResolverError, TargetState,
    };
    use crate::domain::agent::AgentKind;
    use crate::domain::lockfile::Lockfile;
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::source::InstallSource;
    use crate::domain::target::TargetPath;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    struct FakeDependencyStore {
        added: RefCell<Vec<(String, String, Vec<String>)>>,
    }

    impl DependencyConfigStore for FakeDependencyStore {
        fn add_dependency(
            &self,
            skill_name: &str,
            source: &str,
            agents: &[String],
        ) -> Result<(), DependencyConfigStoreError> {
            self.added.borrow_mut().push((
                skill_name.to_owned(),
                source.to_owned(),
                agents.to_vec(),
            ));
            Ok(())
        }

        fn add_dependency_agents(
            &self,
            _skill_name: &str,
            _agents: &[String],
        ) -> Result<Vec<String>, DependencyConfigStoreError> {
            Ok(Vec::new())
        }

        fn remove_dependency(&self, _skill_name: &str) -> Result<(), DependencyConfigStoreError> {
            Ok(())
        }

        fn remove_dependency_agents(
            &self,
            _skill_name: &str,
            _agents: &[String],
        ) -> Result<Vec<String>, DependencyConfigStoreError> {
            Ok(Vec::new())
        }
    }

    struct FakeInstaller;

    impl SkillInstaller for FakeInstaller {
        fn install_skill(
            &self,
            source: &InstallSource,
            _destination: &Path,
            _skill_name: &str,
        ) -> Result<InstalledSkillSource, SkillInstallError> {
            Ok(InstalledSkillSource {
                label: "installed".to_owned(),
                resolved_source: source.clone(),
            })
        }
    }

    struct FakeFs {
        created: Cell<usize>,
    }

    impl SourceStore for FakeFs {
        fn source_exists(&self, _source: &SourcePath) -> Result<bool, SourceStoreError> {
            Ok(true)
        }
    }

    impl LinkStore for FakeFs {
        fn inspect_target(
            &self,
            _target: &TargetPath,
            _expected_source: &SourcePath,
        ) -> Result<TargetState, LinkStoreError> {
            Ok(TargetState::DirectoryConflict)
        }
    }

    impl LinkApplier for FakeFs {
        fn create_symlink(
            &self,
            _source: &SourcePath,
            _target: &TargetPath,
        ) -> Result<(), LinkApplyError> {
            self.created.set(self.created.get() + 1);
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

    struct FakeTargetResolver;

    impl TargetResolver for FakeTargetResolver {
        fn resolve_agent_target(
            &self,
            _agent: &AgentKind,
            _scope: Scope,
            _target_dir_override: Option<&Path>,
        ) -> Result<TargetPath, TargetResolverError> {
            TargetPath::new("targets").map_err(|error| TargetResolverError::Resolve {
                agent: "pi".to_owned(),
                scope: Scope::Project,
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
                scope: Scope::Project,
                target_dir: None,
            },
        );
        ResolvedConfig {
            skill_dir: SourcePath::new("skills").unwrap(),
            agents,
            skills: vec![ResolvedSkill {
                name: SkillName::new("review").unwrap(),
                source: SourcePath::new("skills/review").unwrap(),
                install_source: Some(InstallSource::Local(PathBuf::from("remote/review"))),
                agents: vec![AgentKind::Pi],
            }],
            default_agents: Vec::new(),
        }
    }

    #[test]
    fn add_workflow_skips_existing_target_conflicts() {
        let dependency_store = FakeDependencyStore::default();
        let installer = FakeInstaller;
        let fs_store = FakeFs {
            created: Cell::new(0),
        };
        let lockfile_store = FakeLockfileStore::default();
        let target_resolver = FakeTargetResolver;

        let report = run_add_workflow(
            vec![AddSelection {
                skill_name: "review".to_owned(),
                source: "owner/repo/skills/review".to_owned(),
            }],
            &["pi".to_owned()],
            || Ok(config()),
            |_config, _plan| {
                Ok(Lockfile {
                    generated_by: "test".to_owned(),
                    generated_at: "test".to_owned(),
                    root: PathBuf::from("."),
                    skills: BTreeMap::new(),
                })
            },
            AddWorkflow {
                dependency_store: &dependency_store,
                installer: &installer,
                fs_store: &fs_store,
                lockfile_store: &lockfile_store,
                target_resolver: &target_resolver,
            },
        )
        .expect("add succeeds with skipped conflict");

        assert_eq!(report.added.len(), 1);
        assert_eq!(report.plan.items.len(), 1);
        assert_eq!(fs_store.created.get(), 0);
        assert!(lockfile_store.written.get());
        assert_eq!(dependency_store.added.borrow().len(), 1);
    }
}
