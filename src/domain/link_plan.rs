use std::path::PathBuf;

use crate::domain::agent::AgentKind;
use crate::domain::skill::{SkillName, SourcePath};
use crate::domain::target::TargetPath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPlan {
    pub items: Vec<LinkPlanItem>,
}

impl LinkPlan {
    pub fn new(items: Vec<LinkPlanItem>) -> Self {
        Self { items }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkOwner {
    pub skill: SkillName,
    pub agent: AgentKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPlanItem {
    pub owners: Vec<LinkOwner>,
    pub source: SourcePath,
    pub target: TargetPath,
    pub action: PlanAction,
}

impl LinkPlanItem {
    pub fn skill_label(&self) -> String {
        let mut values = self
            .owners
            .iter()
            .map(|owner| owner.skill.as_str())
            .collect::<Vec<_>>();
        values.sort_unstable();
        values.dedup();
        values.join(", ")
    }

    pub fn agent_label(&self) -> String {
        let mut values = self
            .owners
            .iter()
            .map(|owner| owner.agent.as_str())
            .collect::<Vec<_>>();
        values.sort_unstable();
        values.dedup();
        values.join(", ")
    }

    pub fn has_owner(&self, skill: &SkillName, agent: &AgentKind) -> bool {
        self.owners
            .iter()
            .any(|owner| &owner.skill == skill && &owner.agent == agent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanAction {
    CreateSymlink,
    AlreadySynced,
    Conflict { reason: ConflictReason },
    DriftedSymlink { actual_source: PathBuf },
    SourceMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
    RegularFile,
    Directory,
    BrokenSymlink,
}

impl std::fmt::Display for ConflictReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::RegularFile => "regular file exists",
            Self::Directory => "directory exists",
            Self::BrokenSymlink => "broken symlink exists",
        };
        formatter.write_str(value)
    }
}
