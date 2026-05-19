use crate::application::ports::{LinkStore, SourceHashStore, TargetState};
use crate::domain::lockfile::Lockfile;

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
        }
    }
}

pub fn check_lockfile(
    lockfile: &Lockfile,
    source_hash_store: &impl SourceHashStore,
    link_store: &impl LinkStore,
) -> CheckReport {
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

        for target in &locked_skill.targets {
            let state = link_store.inspect_target(&target.path, &locked_skill.source);
            match state {
                Ok(TargetState::SymlinkToExpectedSource) => {}
                Ok(TargetState::Missing) => problems.push(CheckProblem::TargetMissing {
                    skill: skill_name.as_str().to_owned(),
                    agent: target.agent.as_str().to_owned(),
                    path: target.path.as_path().display().to_string(),
                }),
                Ok(TargetState::SymlinkToUnexpectedSource { actual_source }) => {
                    problems.push(CheckProblem::TargetUnexpectedSymlink {
                        skill: skill_name.as_str().to_owned(),
                        agent: target.agent.as_str().to_owned(),
                        path: target.path.as_path().display().to_string(),
                        actual_source: actual_source.display().to_string(),
                    });
                }
                Ok(TargetState::RegularFileConflict) => {
                    problems.push(CheckProblem::TargetConflict {
                        skill: skill_name.as_str().to_owned(),
                        agent: target.agent.as_str().to_owned(),
                        path: target.path.as_path().display().to_string(),
                        reason: "regular file exists".to_owned(),
                    })
                }
                Ok(TargetState::DirectoryConflict) => problems.push(CheckProblem::TargetConflict {
                    skill: skill_name.as_str().to_owned(),
                    agent: target.agent.as_str().to_owned(),
                    path: target.path.as_path().display().to_string(),
                    reason: "directory exists".to_owned(),
                }),
                Ok(TargetState::BrokenSymlink { actual_source }) => {
                    problems.push(CheckProblem::BrokenSymlink {
                        skill: skill_name.as_str().to_owned(),
                        agent: target.agent.as_str().to_owned(),
                        path: target.path.as_path().display().to_string(),
                        actual_source: actual_source.display().to_string(),
                    });
                }
                Err(error) => problems.push(CheckProblem::InspectFailed {
                    skill: skill_name.as_str().to_owned(),
                    agent: target.agent.as_str().to_owned(),
                    message: error.to_string(),
                }),
            }
        }
    }

    CheckReport { problems }
}

#[cfg(test)]
mod tests {
    use super::{check_lockfile, CheckProblem};
    use crate::application::ports::{
        LinkStore, LinkStoreError, SourceHash, SourceHashStore, SourceHashStoreError, TargetState,
    };
    use crate::domain::agent::AgentKind;
    use crate::domain::lockfile::{Digest, LinkType, LockedSkill, LockedTarget, Lockfile};
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

    fn lockfile() -> Lockfile {
        let mut skills = BTreeMap::new();
        skills.insert(
            SkillName::new("review").unwrap(),
            LockedSkill {
                source: SourcePath::new("skills/review").unwrap(),
                install_source: None,
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
