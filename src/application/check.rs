use crate::application::config::ResolvedConfig;
use crate::application::ports::{LinkStore, SourceHashStore, TargetState};
use crate::domain::link_plan::LinkPlan;
use crate::domain::lockfile::Lockfile;
use crate::domain::target::TargetPath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub problems: Vec<CheckProblem>,
}

impl CheckReport {
    pub fn is_success(&self) -> bool {
        self.problems.is_empty()
    }

    pub fn display_lines(&self) -> Vec<String> {
        if self.problems.is_empty() {
            return vec!["check passed".to_owned()];
        }

        self.problems
            .iter()
            .map(CheckProblem::display_line)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckProblem {
    SourceHashDrift {
        skill: String,
        expected: String,
        actual: String,
    },
    TargetMissing {
        skill: String,
        agent: String,
        path: String,
    },
    TargetUnexpectedSymlink {
        skill: String,
        agent: String,
        path: String,
        actual_source: String,
    },
    TargetConflict {
        skill: String,
        agent: String,
        path: String,
        reason: String,
    },
    BrokenSymlink {
        skill: String,
        agent: String,
        path: String,
        actual_source: String,
    },
    InspectFailed {
        skill: String,
        agent: String,
        message: String,
    },
    HashFailed {
        skill: String,
        message: String,
    },
    IncludeMismatch {
        skill: String,
        expected: String,
        actual: String,
    },
}

impl CheckProblem {
    pub fn display_line(&self) -> String {
        match self {
            Self::SourceHashDrift {
                skill,
                expected,
                actual,
            } => format!("source drift: {skill} expected {expected} but got {actual}"),
            Self::TargetMissing { skill, agent, path } => {
                format!("target missing: skill={skill}, agent={agent}, path={path}")
            }
            Self::TargetUnexpectedSymlink {
                skill,
                agent,
                path,
                actual_source,
            } => format!(
                "target drift: skill={skill}, agent={agent}, path={path}, actual={actual_source}"
            ),
            Self::TargetConflict {
                skill,
                agent,
                path,
                reason,
            } => format!("target conflict: skill={skill}, agent={agent}, path={path}, {reason}"),
            Self::BrokenSymlink {
                skill,
                agent,
                path,
                actual_source,
            } => format!(
                "broken symlink: skill={skill}, agent={agent}, path={path}, target={actual_source}"
            ),
            Self::InspectFailed {
                skill,
                agent,
                message,
            } => format!("inspect failed: skill={skill}, agent={agent}, {message}"),
            Self::HashFailed { skill, message } => {
                format!("hash failed: skill={skill}, {message}")
            }
            Self::IncludeMismatch {
                skill,
                expected,
                actual,
            } => format!("include mismatch: skill={skill}, config={expected}, lockfile={actual}"),
        }
    }
}

/// A set of related check problems that share a single fix hint, for grouped CLI output.
///
/// Grouping keeps `doctor`/`check` output readable: instead of repeating the same
/// remediation hint on every line, related problems are collected under one header
/// with one hint. `count` is the number of underlying problems (which may differ from
/// `lines.len()` when lines include sub-group headers, as with target conflicts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemGroup {
    pub title: String,
    pub count: usize,
    pub hint: String,
    pub lines: Vec<String>,
}

/// Group check problems by problem type into display sections, each with one fix hint.
///
/// This is pure formatting: it never inspects the filesystem and preserves the input
/// problem count across all returned groups. Target conflicts are summarized by agent
/// and parent target directory, with the conflicting skills listed underneath.
pub fn group_check_problems(problems: &[CheckProblem]) -> Vec<ProblemGroup> {
    let mut groups = Vec::new();

    let source_drift: Vec<String> = problems
        .iter()
        .filter_map(|problem| match problem {
            CheckProblem::SourceHashDrift {
                skill,
                expected,
                actual,
            } => Some(format!("{skill}: expected {expected}, got {actual}")),
            _ => None,
        })
        .collect();
    push_group(
        &mut groups,
        "Source drift",
        "run `sksync apply` to relink, or `sksync update` to record the new source",
        source_drift,
    );

    let include_mismatch: Vec<String> = problems
        .iter()
        .filter_map(|problem| match problem {
            CheckProblem::IncludeMismatch {
                skill,
                expected,
                actual,
            } => Some(format!("{skill}: config {expected}, lockfile {actual}")),
            _ => None,
        })
        .collect();
    push_group(
        &mut groups,
        "Include mismatch",
        "run `sksync apply` to apply the configured include filter",
        include_mismatch,
    );

    let target_missing: Vec<String> = problems
        .iter()
        .filter_map(|problem| match problem {
            CheckProblem::TargetMissing { skill, agent, path } => {
                Some(format!("{skill} · {agent} · {path}"))
            }
            _ => None,
        })
        .collect();
    push_group(
        &mut groups,
        "Target missing",
        "run `sksync apply` to create the missing link(s)",
        target_missing,
    );

    let target_drift: Vec<String> = problems
        .iter()
        .filter_map(|problem| match problem {
            CheckProblem::TargetUnexpectedSymlink {
                skill,
                agent,
                path,
                actual_source,
            } => Some(format!("{skill} · {agent} · {path} → {actual_source}")),
            _ => None,
        })
        .collect();
    push_group(
        &mut groups,
        "Target drift",
        "inspect with `sksync plan`, then `sksync apply --force` if the drift is safe to overwrite",
        target_drift,
    );

    if let Some(group) = group_target_conflicts(problems) {
        groups.push(group);
    }

    let broken_symlinks: Vec<String> = problems
        .iter()
        .filter_map(|problem| match problem {
            CheckProblem::BrokenSymlink {
                skill,
                agent,
                path,
                actual_source,
            } => Some(format!("{skill} · {agent} · {path} → {actual_source}")),
            _ => None,
        })
        .collect();
    push_group(
        &mut groups,
        "Broken symlinks",
        "run `sksync apply` to recreate the link, or remove the dangling symlink",
        broken_symlinks,
    );

    let inspect_failed: Vec<String> = problems
        .iter()
        .filter_map(|problem| match problem {
            CheckProblem::InspectFailed {
                skill,
                agent,
                message,
            } => Some(format!("{skill} · {agent}: {message}")),
            _ => None,
        })
        .collect();
    push_group(
        &mut groups,
        "Inspect failed",
        "check filesystem permissions, then re-run `sksync doctor`",
        inspect_failed,
    );

    let hash_failed: Vec<String> = problems
        .iter()
        .filter_map(|problem| match problem {
            CheckProblem::HashFailed { skill, message } => Some(format!("{skill}: {message}")),
            _ => None,
        })
        .collect();
    push_group(
        &mut groups,
        "Hash failed",
        "verify the source path exists and is readable, then `sksync update`",
        hash_failed,
    );

    groups
}

fn push_group(groups: &mut Vec<ProblemGroup>, title: &str, hint: &str, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    groups.push(ProblemGroup {
        title: title.to_owned(),
        count: lines.len(),
        hint: hint.to_owned(),
        lines,
    });
}

/// Summarize target conflicts by agent and parent target directory.
///
/// Conflicting skills are listed under each `agent · parent-dir` header so the shared
/// directory and fix hint are not repeated on every line.
fn group_target_conflicts(problems: &[CheckProblem]) -> Option<ProblemGroup> {
    let mut by_location: std::collections::BTreeMap<(String, String), Vec<String>> =
        std::collections::BTreeMap::new();
    let mut count = 0;

    for problem in problems {
        if let CheckProblem::TargetConflict {
            skill,
            agent,
            path,
            reason,
        } = problem
        {
            let parent = std::path::Path::new(path)
                .parent()
                .map(|parent| parent.display().to_string())
                .filter(|parent| !parent.is_empty())
                .unwrap_or_else(|| ".".to_owned());
            by_location
                .entry((agent.clone(), parent))
                .or_default()
                .push(format!("{skill} ({reason})"));
            count += 1;
        }
    }

    if count == 0 {
        return None;
    }

    let mut lines = Vec::new();
    for ((agent, parent), skills) in by_location {
        lines.push(format!("{agent} · {parent}"));
        for skill in skills {
            lines.push(format!("  {skill}"));
        }
    }

    Some(ProblemGroup {
        title: "Target conflicts".to_owned(),
        count,
        hint: "inspect with `sksync plan`; remove or relocate the conflicting path(s), then `sksync apply`"
            .to_owned(),
        lines,
    })
}

pub fn check_lockfile(
    lockfile: &Lockfile,
    source_hash_store: &impl SourceHashStore,
    link_store: &impl LinkStore,
) -> CheckReport {
    let mut problems = check_source_hashes(lockfile, source_hash_store);

    for (skill_name, locked_skill) in &lockfile.skills {
        for target in &locked_skill.targets {
            inspect_target(
                &mut problems,
                skill_name.as_str(),
                target.agent.as_str(),
                &target.path,
                &locked_skill.source,
                link_store,
            );
        }
    }

    CheckReport { problems }
}

pub fn check_lockfile_with_plan(
    lockfile: &Lockfile,
    plan: &LinkPlan,
    source_hash_store: &impl SourceHashStore,
    link_store: &impl LinkStore,
) -> CheckReport {
    let mut problems = check_source_hashes(lockfile, source_hash_store);

    inspect_plan_targets(&mut problems, plan, link_store);

    CheckReport { problems }
}

pub fn check_lockfile_with_config_and_plan(
    config: &ResolvedConfig,
    lockfile: &Lockfile,
    plan: &LinkPlan,
    source_hash_store: &impl SourceHashStore,
    link_store: &impl LinkStore,
) -> CheckReport {
    let mut problems = check_include_filters(config, lockfile);
    problems.extend(check_source_hashes(lockfile, source_hash_store));

    inspect_plan_targets(&mut problems, plan, link_store);

    CheckReport { problems }
}

fn inspect_plan_targets(
    problems: &mut Vec<CheckProblem>,
    plan: &LinkPlan,
    link_store: &impl LinkStore,
) {
    for item in &plan.items {
        inspect_target(
            problems,
            &item.skill_label(),
            &item.agent_label(),
            &item.target,
            &item.source,
            link_store,
        );
    }
}

fn check_include_filters(config: &ResolvedConfig, lockfile: &Lockfile) -> Vec<CheckProblem> {
    let mut problems = Vec::new();
    for skill in &config.skills {
        let Some(locked) = lockfile.skills.get(&skill.name) else {
            continue;
        };
        if skill.include != locked.include {
            problems.push(CheckProblem::IncludeMismatch {
                skill: skill.name.as_str().to_owned(),
                expected: format_include(skill.include.as_ref()),
                actual: format_include(locked.include.as_ref()),
            });
        }
    }
    problems
}

fn format_include(include: Option<&crate::domain::package_filter::PackageFilter>) -> String {
    include
        .map(|include| include.patterns().join(", "))
        .unwrap_or_else(|| "<full package>".to_owned())
}

fn check_source_hashes(
    lockfile: &Lockfile,
    source_hash_store: &impl SourceHashStore,
) -> Vec<CheckProblem> {
    let mut problems = Vec::new();

    for (skill_name, locked_skill) in &lockfile.skills {
        match source_hash_store.hash_source(&locked_skill.source) {
            Ok(actual) if actual.hash != locked_skill.hash => {
                problems.push(CheckProblem::SourceHashDrift {
                    skill: skill_name.as_str().to_owned(),
                    expected: locked_skill.hash.as_str().to_owned(),
                    actual: actual.hash.as_str().to_owned(),
                });
            }
            Ok(_) => {}
            Err(error) => problems.push(CheckProblem::HashFailed {
                skill: skill_name.as_str().to_owned(),
                message: error.to_string(),
            }),
        }
    }

    problems
}

fn inspect_target(
    problems: &mut Vec<CheckProblem>,
    skill: &str,
    agent: &str,
    target: &TargetPath,
    expected_source: &crate::domain::skill::SourcePath,
    link_store: &impl LinkStore,
) {
    let state = link_store.inspect_target(target, expected_source);
    match state {
        Ok(TargetState::SymlinkToExpectedSource) => {}
        Ok(TargetState::Missing) => problems.push(CheckProblem::TargetMissing {
            skill: skill.to_owned(),
            agent: agent.to_owned(),
            path: target.as_path().display().to_string(),
        }),
        Ok(TargetState::SymlinkToUnexpectedSource { actual_source }) => {
            problems.push(CheckProblem::TargetUnexpectedSymlink {
                skill: skill.to_owned(),
                agent: agent.to_owned(),
                path: target.as_path().display().to_string(),
                actual_source: actual_source.display().to_string(),
            });
        }
        Ok(TargetState::RegularFileConflict) => problems.push(CheckProblem::TargetConflict {
            skill: skill.to_owned(),
            agent: agent.to_owned(),
            path: target.as_path().display().to_string(),
            reason: "regular file exists".to_owned(),
        }),
        Ok(TargetState::DirectoryConflict) => problems.push(CheckProblem::TargetConflict {
            skill: skill.to_owned(),
            agent: agent.to_owned(),
            path: target.as_path().display().to_string(),
            reason: "directory exists".to_owned(),
        }),
        Ok(TargetState::BrokenSymlink { actual_source }) => {
            problems.push(CheckProblem::BrokenSymlink {
                skill: skill.to_owned(),
                agent: agent.to_owned(),
                path: target.as_path().display().to_string(),
                actual_source: actual_source.display().to_string(),
            });
        }
        Err(error) => problems.push(CheckProblem::InspectFailed {
            skill: skill.to_owned(),
            agent: agent.to_owned(),
            message: error.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        check_lockfile, check_lockfile_with_config_and_plan, check_lockfile_with_plan, CheckProblem,
    };
    use crate::application::config::{ResolvedConfig, ResolvedSkill};
    use crate::application::ports::{
        LinkStore, LinkStoreError, SourceHash, SourceHashStore, SourceHashStoreError, TargetState,
    };
    use crate::domain::agent::AgentKind;
    use crate::domain::link_plan::{LinkOwner, LinkPlan, LinkPlanItem, PlanAction};
    use crate::domain::lockfile::{Digest, LinkType, LockedSkill, LockedTarget, Lockfile};
    use crate::domain::package_filter::PackageFilter;
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::target::TargetPath;
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    struct FakeHashStore {
        hash: &'static str,
    }

    impl SourceHashStore for FakeHashStore {
        fn hash_source(&self, _source: &SourcePath) -> Result<SourceHash, SourceHashStoreError> {
            Ok(SourceHash {
                hash: Digest::new(self.hash).unwrap(),
            })
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

    fn config_with_manifest_only() -> ResolvedConfig {
        ResolvedConfig {
            skill_dir: SourcePath::new("skills").unwrap(),
            agents: BTreeMap::new(),
            skills: vec![ResolvedSkill {
                name: SkillName::new("review").unwrap(),
                source: SourcePath::new("skills/review").unwrap(),
                install_source: None,
                include: Some(PackageFilter::manifest_only()),
                agents: vec![AgentKind::Pi],
            }],
            default_agents: Vec::new(),
        }
    }

    fn lockfile() -> Lockfile {
        let mut skills = BTreeMap::new();
        skills.insert(
            SkillName::new("review").unwrap(),
            LockedSkill {
                source: SourcePath::new("skills/review").unwrap(),
                install_source: None,
                include: None,
                hash: Digest::new("sha256-expected").unwrap(),
                files: Vec::new(),
                targets: vec![LockedTarget {
                    agent: AgentKind::Pi,
                    scope: Scope::Project,
                    path: TargetPath::new(".pi/agent/skills/review").unwrap(),
                    link_type: LinkType::Symlink,
                }],
            },
        );
        Lockfile {
            generated_by: "sksync@test".to_owned(),
            generated_at: "test".to_owned(),
            root: PathBuf::from("."),
            skills,
        }
    }

    #[test]
    fn synced_state_succeeds() {
        let report = check_lockfile(
            &lockfile(),
            &FakeHashStore {
                hash: "sha256-expected",
            },
            &FakeLinkStore {
                state: TargetState::SymlinkToExpectedSource,
            },
        );

        assert!(report.is_success());
    }

    #[test]
    fn source_hash_change_is_reported_as_drift() {
        let report = check_lockfile(
            &lockfile(),
            &FakeHashStore {
                hash: "sha256-actual",
            },
            &FakeLinkStore {
                state: TargetState::SymlinkToExpectedSource,
            },
        );

        assert!(matches!(
            report.problems[0],
            CheckProblem::SourceHashDrift { .. }
        ));
    }

    #[test]
    fn target_missing_is_reported() {
        let report = check_lockfile(
            &lockfile(),
            &FakeHashStore {
                hash: "sha256-expected",
            },
            &FakeLinkStore {
                state: TargetState::Missing,
            },
        );

        assert!(matches!(
            report.problems[0],
            CheckProblem::TargetMissing { .. }
        ));
    }

    #[test]
    fn include_mismatch_is_reported() {
        let plan = LinkPlan::new(vec![LinkPlanItem {
            owners: vec![LinkOwner {
                skill: SkillName::new("review").unwrap(),
                agent: AgentKind::Pi,
            }],
            source: SourcePath::new("skills/review").unwrap(),
            target: TargetPath::new(".pi/agent/skills/review").unwrap(),
            action: PlanAction::AlreadySynced,
        }]);

        let report = check_lockfile_with_config_and_plan(
            &config_with_manifest_only(),
            &lockfile(),
            &plan,
            &FakeHashStore {
                hash: "sha256-expected",
            },
            &FakeLinkStore {
                state: TargetState::SymlinkToExpectedSource,
            },
        );

        assert!(matches!(
            report.problems[0],
            CheckProblem::IncludeMismatch { .. }
        ));
    }

    #[test]
    fn planned_target_missing_is_reported_when_lockfile_has_no_targets() {
        let mut lockfile = lockfile();
        lockfile
            .skills
            .get_mut(&SkillName::new("review").unwrap())
            .unwrap()
            .targets
            .clear();
        let plan = LinkPlan::new(vec![LinkPlanItem {
            owners: vec![LinkOwner {
                skill: SkillName::new("review").unwrap(),
                agent: AgentKind::Pi,
            }],
            source: SourcePath::new("skills/review").unwrap(),
            target: TargetPath::new(".pi/agent/skills/review").unwrap(),
            action: PlanAction::CreateSymlink,
        }]);

        let report = check_lockfile_with_plan(
            &lockfile,
            &plan,
            &FakeHashStore {
                hash: "sha256-expected",
            },
            &FakeLinkStore {
                state: TargetState::Missing,
            },
        );

        assert!(matches!(
            report.problems[0],
            CheckProblem::TargetMissing { .. }
        ));
    }

    #[test]
    fn shared_target_problem_lists_all_owners() {
        let plan = LinkPlan::new(vec![LinkPlanItem {
            owners: vec![
                LinkOwner {
                    skill: SkillName::new("review").unwrap(),
                    agent: AgentKind::Pi,
                },
                LinkOwner {
                    skill: SkillName::new("review").unwrap(),
                    agent: AgentKind::custom("universal").unwrap(),
                },
            ],
            source: SourcePath::new("skills/review").unwrap(),
            target: TargetPath::new(".agents/skills/review").unwrap(),
            action: PlanAction::CreateSymlink,
        }]);

        let links = CountingLinkStore {
            state: TargetState::Missing,
            inspections: Cell::new(0),
        };
        let report = check_lockfile_with_plan(
            &lockfile(),
            &plan,
            &FakeHashStore {
                hash: "sha256-expected",
            },
            &links,
        );

        assert_eq!(links.inspections.get(), 1);
        assert!(matches!(
            &report.problems[0],
            CheckProblem::TargetMissing { agent, .. } if agent == "pi, universal"
        ));
    }

    #[test]
    fn target_conflicts_are_grouped_by_agent_and_parent_with_single_hint() {
        use super::group_check_problems;

        let problems = vec![
            CheckProblem::TargetConflict {
                skill: "review".to_owned(),
                agent: "pi".to_owned(),
                path: ".pi/agent/skills/review".to_owned(),
                reason: "regular file exists".to_owned(),
            },
            CheckProblem::TargetConflict {
                skill: "tdd".to_owned(),
                agent: "pi".to_owned(),
                path: ".pi/agent/skills/tdd".to_owned(),
                reason: "directory exists".to_owned(),
            },
            CheckProblem::TargetConflict {
                skill: "review".to_owned(),
                agent: "codex".to_owned(),
                path: ".codex/skills/review".to_owned(),
                reason: "regular file exists".to_owned(),
            },
        ];

        let groups = group_check_problems(&problems);
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.title, "Target conflicts");
        // Count reflects every underlying problem, not the number of display lines.
        assert_eq!(group.count, 3);
        // A single shared fix hint, not one per item.
        assert!(group.hint.contains("sksync plan"));

        // Two location headers (pi and codex), with skills listed underneath.
        let headers: Vec<&String> = group
            .lines
            .iter()
            .filter(|line| !line.starts_with("  "))
            .collect();
        assert_eq!(headers.len(), 2);
        assert!(group
            .lines
            .iter()
            .any(|line| line == "pi · .pi/agent/skills"));
        assert!(group
            .lines
            .iter()
            .any(|line| line.trim() == "review (regular file exists)"));
    }

    #[test]
    fn group_check_problems_preserves_total_count_across_groups() {
        use super::group_check_problems;

        let problems = vec![
            CheckProblem::SourceHashDrift {
                skill: "review".to_owned(),
                expected: "sha256-a".to_owned(),
                actual: "sha256-b".to_owned(),
            },
            CheckProblem::TargetMissing {
                skill: "tdd".to_owned(),
                agent: "pi".to_owned(),
                path: ".pi/agent/skills/tdd".to_owned(),
            },
            CheckProblem::TargetConflict {
                skill: "review".to_owned(),
                agent: "pi".to_owned(),
                path: ".pi/agent/skills/review".to_owned(),
                reason: "regular file exists".to_owned(),
            },
        ];

        let groups = group_check_problems(&problems);
        let total: usize = groups.iter().map(|group| group.count).sum();
        assert_eq!(total, problems.len());
    }

    #[test]
    fn broken_symlink_is_reported() {
        let report = check_lockfile(
            &lockfile(),
            &FakeHashStore {
                hash: "sha256-expected",
            },
            &FakeLinkStore {
                state: TargetState::BrokenSymlink {
                    actual_source: PathBuf::from("missing"),
                },
            },
        );

        assert!(matches!(
            report.problems[0],
            CheckProblem::BrokenSymlink { .. }
        ));
    }
}
