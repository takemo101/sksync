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
        ApplyOptions { force: false },
    )?;

    Ok(AddWorkflowReport {
        added,
        update_report,
        plan,
    })
}
