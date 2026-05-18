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

    pub fn display_lines(&self) -> Vec<String> {
        self.items.iter().map(LinkPlanItem::display_line).collect()
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

impl LinkPlanItem {
    pub fn display_line(&self) -> String {
        match &self.action {
            PlanAction::CreateSymlink => format!(
                "create symlink: {} -> {} ({})",
                self.target.as_path().display(),
                self.source.as_path().display(),
                self.label()
            ),
            PlanAction::AlreadySynced => format!(
                "already synced: {} ({})",
                self.target.as_path().display(),
                self.label()
            ),
            PlanAction::Conflict { reason } => format!(
                "conflict: {} ({}, {reason})",
                self.target.as_path().display(),
                self.label()
            ),
            PlanAction::DriftedSymlink { actual_source } => format!(
                "drifted symlink: {} points to {} but expected {} ({})",
                self.target.as_path().display(),
                actual_source.display(),
                self.source.as_path().display(),
                self.label()
            ),
            PlanAction::SourceMissing => format!(
                "source missing: {} ({})",
                self.source.as_path().display(),
                self.label()
            ),
        }
    }

    fn label(&self) -> String {
        format!("skill={}, agent={}", self.skill, self.agent.as_str())
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
