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
pub struct LinkPlanItem {
    pub skill: SkillName,
    pub agent: AgentKind,
    pub source: SourcePath,
    pub target: TargetPath,
    pub action: PlanAction,
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
