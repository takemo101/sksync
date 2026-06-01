use crate::application::config::ResolvedConfig;
use crate::application::ports::{LinkStore, SourceHashStore, TargetState};
use crate::domain::agent::AgentKind;
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
                &target.agent,
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
            item.skill.as_str(),
            &item.agent,
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
    agent: &AgentKind,
    target: &TargetPath,
    expected_source: &crate::domain::skill::SourcePath,
    link_store: &impl LinkStore,
) {
    let state = link_store.inspect_target(target, expected_source);
    match state {
        Ok(TargetState::SymlinkToExpectedSource) => {}
        Ok(TargetState::Missing) => problems.push(CheckProblem::TargetMissing {
            skill: skill.to_owned(),
            agent: agent.as_str().to_owned(),
            path: target.as_path().display().to_string(),
        }),
        Ok(TargetState::SymlinkToUnexpectedSource { actual_source }) => {
            problems.push(CheckProblem::TargetUnexpectedSymlink {
                skill: skill.to_owned(),
                agent: agent.as_str().to_owned(),
                path: target.as_path().display().to_string(),
                actual_source: actual_source.display().to_string(),
            });
        }
        Ok(TargetState::RegularFileConflict) => problems.push(CheckProblem::TargetConflict {
            skill: skill.to_owned(),
            agent: agent.as_str().to_owned(),
            path: target.as_path().display().to_string(),
            reason: "regular file exists".to_owned(),
        }),
        Ok(TargetState::DirectoryConflict) => problems.push(CheckProblem::TargetConflict {
            skill: skill.to_owned(),
            agent: agent.as_str().to_owned(),
            path: target.as_path().display().to_string(),
            reason: "directory exists".to_owned(),
        }),
        Ok(TargetState::BrokenSymlink { actual_source }) => {
            problems.push(CheckProblem::BrokenSymlink {
                skill: skill.to_owned(),
                agent: agent.as_str().to_owned(),
                path: target.as_path().display().to_string(),
                actual_source: actual_source.display().to_string(),
            });
        }
        Err(error) => problems.push(CheckProblem::InspectFailed {
            skill: skill.to_owned(),
            agent: agent.as_str().to_owned(),
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
    use crate::domain::link_plan::{LinkPlan, LinkPlanItem, PlanAction};
    use crate::domain::lockfile::{Digest, LinkType, LockedSkill, LockedTarget, Lockfile};
    use crate::domain::package_filter::PackageFilter;
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::target::TargetPath;
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
            skill: SkillName::new("review").unwrap(),
            agent: AgentKind::Pi,
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
            skill: SkillName::new("review").unwrap(),
            agent: AgentKind::Pi,
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
