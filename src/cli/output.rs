use std::io::Write;

use serde::Serialize;
use thiserror::Error;

// P09–P12 consume these output-only APIs. Remove each narrow allowance as its
// command adapter lands; the application/domain reports remain presentation-free.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JsonCommand {
    List,
    Plan,
    Check,
    #[allow(dead_code)] // P12
    Outdated,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputScope {
    Project,
    Global,
}

#[derive(Debug, Serialize)]
pub struct OutputError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonEnvelope<T> {
    pub schema_version: u32,
    pub command: JsonCommand,
    pub scope: OutputScope,
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<OutputError>,
}

#[derive(Debug, Error)]
pub enum OutputWriteError {
    #[error("failed to serialize JSON output: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write JSON output: {0}")]
    Write(#[source] std::io::Error),
}

impl OutputWriteError {
    #[allow(dead_code)]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Serialize(_) => codes::SERIALIZATION_FAILED,
            Self::Write(_) => codes::IO_ERROR,
        }
    }
}

pub fn write_json<W: Write, T: Serialize>(
    writer: &mut W,
    envelope: &JsonEnvelope<T>,
) -> Result<(), OutputWriteError> {
    let mut bytes = serde_json::to_vec(envelope).map_err(OutputWriteError::Serialize)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).map_err(OutputWriteError::Write)
}

/// A handler has already emitted its failed report. Control flow only: never print.
#[derive(Debug, Error)]
#[error("command failure already rendered")]
pub(crate) struct RenderedFailure {
    pub exit_code: u8,
}

/// Stable machine codes; adapters select these by typed causes, not message text.
pub mod codes {
    pub const CONFIG_NOT_FOUND: &str = "CONFIG_NOT_FOUND";
    pub const INVALID_CONFIG: &str = "INVALID_CONFIG";
    pub const LOCKFILE_NOT_FOUND: &str = "LOCKFILE_NOT_FOUND";
    pub const INVALID_LOCKFILE: &str = "INVALID_LOCKFILE";
    pub const TARGET_RESOLUTION_FAILED: &str = "TARGET_RESOLUTION_FAILED";
    pub const INSPECTION_FAILED: &str = "INSPECTION_FAILED";
    // Consumed by the command adapters in P09–P12.
    #[allow(dead_code)]
    pub const REMOTE_QUERY_FAILED: &str = "REMOTE_QUERY_FAILED";
    pub const CHECK_FAILED: &str = "CHECK_FAILED";
    pub const IO_ERROR: &str = "IO_ERROR";
    pub const SERIALIZATION_FAILED: &str = "SERIALIZATION_FAILED";
    pub const INTERNAL_ERROR: &str = "INTERNAL_ERROR";
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListData {
    skills: Vec<ListSkill>,
    lockfile_status: ListLockfileStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum ListLockfileStatus {
    Available,
    Missing,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListSkill {
    name: String,
    source: std::path::PathBuf,
    install_source: Option<ListInstallSource>,
    include: Option<Vec<String>>,
    locked_hash: Option<String>,
    targets: Vec<ListTarget>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ListInstallSource {
    Local {
        path: std::path::PathBuf,
    },
    Git {
        url: String,
        path: std::path::PathBuf,
        #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
    },
}

#[derive(Debug, Serialize)]
struct ListTarget {
    agent: String,
    target: Option<std::path::PathBuf>,
    status: ListTargetStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<OutputError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum ListTargetStatus {
    Synced,
    Missing,
    Drifted,
    Conflict,
    BrokenSymlink,
    SourceMissing,
    ResolveFailed,
    InspectFailed,
}

pub fn list_target_error(
    state: &crate::application::list::ListedTargetState,
) -> Option<OutputError> {
    use crate::application::list::ListedTargetState;
    let (code, message) = match state {
        ListedTargetState::ResolveFailed(message) => (codes::TARGET_RESOLUTION_FAILED, message),
        ListedTargetState::InspectFailed(message) => (codes::INSPECTION_FAILED, message),
        _ => return None,
    };
    Some(OutputError {
        code: code.to_owned(),
        message: message.clone(),
        hint: None,
    })
}

impl From<&crate::application::list::ListReport> for ListData {
    fn from(report: &crate::application::list::ListReport) -> Self {
        use crate::application::list::{ListLockfileStatus as ReportStatus, ListedTargetState};
        use crate::domain::source::InstallSource;
        Self {
            lockfile_status: match report.lockfile_status {
                ReportStatus::Available => ListLockfileStatus::Available,
                ReportStatus::Missing => ListLockfileStatus::Missing,
            },
            skills: report
                .skills
                .iter()
                .map(|skill| ListSkill {
                    name: skill.name.clone(),
                    source: skill.source.clone(),
                    install_source: skill.install_source.as_ref().map(|source| match source {
                        InstallSource::Local(path) => {
                            ListInstallSource::Local { path: path.clone() }
                        }
                        InstallSource::Git(source) => ListInstallSource::Git {
                            url: source.url.clone(),
                            path: source.path.clone(),
                            reference: source.reference.clone(),
                        },
                    }),
                    include: skill
                        .include
                        .as_ref()
                        .map(|filter| filter.patterns().to_vec()),
                    locked_hash: skill.locked_hash.clone(),
                    targets: skill
                        .targets
                        .iter()
                        .map(|target| ListTarget {
                            agent: target.agent.clone(),
                            target: target.target.clone(),
                            status: match target.state {
                                ListedTargetState::Synced => ListTargetStatus::Synced,
                                ListedTargetState::Missing => ListTargetStatus::Missing,
                                ListedTargetState::Drifted => ListTargetStatus::Drifted,
                                ListedTargetState::Conflict => ListTargetStatus::Conflict,
                                ListedTargetState::BrokenSymlink => ListTargetStatus::BrokenSymlink,
                                ListedTargetState::SourceMissing => ListTargetStatus::SourceMissing,
                                ListedTargetState::ResolveFailed(_) => {
                                    ListTargetStatus::ResolveFailed
                                }
                                ListedTargetState::InspectFailed(_) => {
                                    ListTargetStatus::InspectFailed
                                }
                            },
                            error: list_target_error(&target.state),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PlanData {
    items: Vec<PlanItem>,
    applicable: bool,
}

#[derive(Debug, Serialize)]
struct PlanItem {
    owners: Vec<PlanOwner>,
    source: std::path::PathBuf,
    target: std::path::PathBuf,
    #[serde(flatten)]
    action: PlanAction,
}

#[derive(Debug, Serialize)]
struct PlanOwner {
    skill: String,
    agent: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "camelCase")]
enum PlanAction {
    CreateSymlink,
    AlreadySynced,
    Conflict {
        reason: PlanConflictReason,
    },
    DriftedSymlink {
        #[serde(rename = "actualSource")]
        actual_source: std::path::PathBuf,
    },
    SourceMissing,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum PlanConflictReason {
    RegularFile,
    Directory,
    BrokenSymlink,
}

impl From<&crate::domain::link_plan::LinkPlan> for PlanData {
    fn from(plan: &crate::domain::link_plan::LinkPlan) -> Self {
        use crate::domain::link_plan::{ConflictReason, PlanAction as Action};
        let mut items = plan
            .items
            .iter()
            .map(|item| {
                let mut owners = item
                    .owners
                    .iter()
                    .map(|owner| PlanOwner {
                        skill: owner.skill.as_str().to_owned(),
                        agent: owner.agent.as_str().to_owned(),
                    })
                    .collect::<Vec<_>>();
                owners.sort_by(|left, right| {
                    (&left.skill, &left.agent).cmp(&(&right.skill, &right.agent))
                });
                PlanItem {
                    owners,
                    source: item.source.as_path().to_path_buf(),
                    target: item.target.as_path().to_path_buf(),
                    action: match &item.action {
                        Action::CreateSymlink => PlanAction::CreateSymlink,
                        Action::AlreadySynced => PlanAction::AlreadySynced,
                        Action::Conflict { reason } => PlanAction::Conflict {
                            reason: match reason {
                                ConflictReason::RegularFile => PlanConflictReason::RegularFile,
                                ConflictReason::Directory => PlanConflictReason::Directory,
                                ConflictReason::BrokenSymlink => PlanConflictReason::BrokenSymlink,
                            },
                        },
                        Action::DriftedSymlink { actual_source } => PlanAction::DriftedSymlink {
                            actual_source: actual_source.clone(),
                        },
                        Action::SourceMissing => PlanAction::SourceMissing,
                    },
                }
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.target.cmp(&right.target));
        Self {
            applicable: plan
                .items
                .iter()
                .all(|item| matches!(item.action, Action::CreateSymlink | Action::AlreadySynced)),
            items,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CheckData<'a> {
    healthy: bool,
    problems: Vec<CheckProblemData<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum CheckProblemData<'a> {
    SourceHashDrift {
        skill: &'a str,
        expected: &'a str,
        actual: &'a str,
    },
    TargetMissing {
        skill: &'a str,
        agent: &'a str,
        path: &'a str,
    },
    TargetUnexpectedSymlink {
        skill: &'a str,
        agent: &'a str,
        path: &'a str,
        #[serde(rename = "actualSource")]
        actual_source: &'a str,
    },
    BrokenSymlink {
        skill: &'a str,
        agent: &'a str,
        path: &'a str,
        #[serde(rename = "actualSource")]
        actual_source: &'a str,
    },
    TargetConflict {
        skill: &'a str,
        agent: &'a str,
        path: &'a str,
        reason: &'a str,
    },
    InspectFailed {
        skill: &'a str,
        agent: &'a str,
        message: &'a str,
    },
    HashFailed {
        skill: &'a str,
        message: &'a str,
    },
    IncludeMismatch {
        skill: &'a str,
        expected: &'a str,
        actual: &'a str,
    },
}

impl<'a> From<&'a crate::application::check::CheckReport> for CheckData<'a> {
    fn from(report: &'a crate::application::check::CheckReport) -> Self {
        use crate::application::check::CheckProblem;
        let mut problems = report
            .problems
            .iter()
            .flat_map(|problem| match problem {
                CheckProblem::SourceHashDrift {
                    skill,
                    expected,
                    actual,
                } => vec![CheckProblemData::SourceHashDrift {
                    skill,
                    expected,
                    actual,
                }],
                CheckProblem::TargetMissing { owners, path, .. } => owners
                    .iter()
                    .map(|owner| CheckProblemData::TargetMissing {
                        skill: owner.skill.as_str(),
                        agent: owner.agent.as_str(),
                        path,
                    })
                    .collect(),
                CheckProblem::TargetUnexpectedSymlink {
                    owners,
                    path,
                    actual_source,
                    ..
                } => owners
                    .iter()
                    .map(|owner| CheckProblemData::TargetUnexpectedSymlink {
                        skill: owner.skill.as_str(),
                        agent: owner.agent.as_str(),
                        path,
                        actual_source,
                    })
                    .collect(),
                CheckProblem::BrokenSymlink {
                    owners,
                    path,
                    actual_source,
                    ..
                } => owners
                    .iter()
                    .map(|owner| CheckProblemData::BrokenSymlink {
                        skill: owner.skill.as_str(),
                        agent: owner.agent.as_str(),
                        path,
                        actual_source,
                    })
                    .collect(),
                CheckProblem::TargetConflict {
                    owners,
                    path,
                    reason,
                    ..
                } => owners
                    .iter()
                    .map(|owner| CheckProblemData::TargetConflict {
                        skill: owner.skill.as_str(),
                        agent: owner.agent.as_str(),
                        path,
                        reason,
                    })
                    .collect(),
                CheckProblem::InspectFailed {
                    owners, message, ..
                } => owners
                    .iter()
                    .map(|owner| CheckProblemData::InspectFailed {
                        skill: owner.skill.as_str(),
                        agent: owner.agent.as_str(),
                        message,
                    })
                    .collect(),
                CheckProblem::HashFailed { skill, message } => {
                    vec![CheckProblemData::HashFailed { skill, message }]
                }
                CheckProblem::IncludeMismatch {
                    skill,
                    expected,
                    actual,
                } => vec![CheckProblemData::IncludeMismatch {
                    skill,
                    expected,
                    actual,
                }],
            })
            .collect::<Vec<_>>();
        problems.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        Self {
            healthy: report.is_success(),
            problems,
        }
    }
}

impl CheckProblemData<'_> {
    fn sort_key(&self) -> (&str, &str, &str, &str) {
        match self {
            Self::SourceHashDrift { skill, .. } => ("sourceHashDrift", *skill, "", ""),
            Self::TargetMissing {
                skill, agent, path, ..
            } => ("targetMissing", *skill, *agent, *path),
            Self::TargetUnexpectedSymlink {
                skill, agent, path, ..
            } => ("targetUnexpectedSymlink", *skill, *agent, *path),
            Self::BrokenSymlink {
                skill, agent, path, ..
            } => ("brokenSymlink", *skill, *agent, *path),
            Self::TargetConflict {
                skill, agent, path, ..
            } => ("targetConflict", *skill, *agent, *path),
            Self::InspectFailed { skill, agent, .. } => ("inspectFailed", *skill, *agent, ""),
            Self::HashFailed { skill, .. } => ("hashFailed", *skill, "", ""),
            Self::IncludeMismatch { skill, .. } => ("includeMismatch", *skill, "", ""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use std::error::Error;
    use std::io::{self, Write};

    fn outdated<T>(data: T) -> JsonEnvelope<T> {
        JsonEnvelope {
            schema_version: 1,
            command: JsonCommand::Outdated,
            scope: OutputScope::Project,
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    #[test]
    fn json_envelope_empty_outdated_is_one_newline_terminated_object() {
        let mut bytes = Vec::new();
        write_json(
            &mut bytes,
            &outdated(serde_json::json!({"rows": [], "problems": []})),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["command"], "outdated");
        assert_eq!(value["scope"], "project");
        assert_eq!(value["ok"], true);
        assert!(value["error"].is_null());
        assert_eq!(value["data"]["rows"], serde_json::json!([]));
        assert_eq!(value["data"]["problems"], serde_json::json!([]));
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert_eq!(bytes.iter().filter(|&&b| b == b'\n').count(), 1);
    }

    #[test]
    fn json_envelope_failure_preserves_nulls_and_omits_absent_hint() {
        for (command, expected_command) in [
            (JsonCommand::List, "list"),
            (JsonCommand::Plan, "plan"),
            (JsonCommand::Check, "check"),
            (JsonCommand::Outdated, "outdated"),
        ] {
            let envelope = JsonEnvelope::<()> {
                schema_version: 1,
                command,
                scope: OutputScope::Global,
                ok: false,
                data: None,
                error: Some(OutputError {
                    code: codes::INTERNAL_ERROR.into(),
                    message: "Failed.".into(),
                    hint: None,
                }),
            };
            let mut bytes = Vec::new();
            write_json(&mut bytes, &envelope).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(value["data"].is_null());
            assert_eq!(value["command"], expected_command);
            assert_eq!(value["scope"], "global");
            assert!(value["error"].get("hint").is_none());
        }
        let error = OutputError {
            code: codes::CHECK_FAILED.into(),
            message: "Unhealthy.".into(),
            hint: Some("Run check.".into()),
        };
        assert_eq!(serde_json::to_value(error).unwrap()["hint"], "Run check.");
    }

    struct FailsSerialization;
    impl Serialize for FailsSerialization {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("injected serialization failure"))
        }
    }

    #[derive(Default)]
    struct RecordingWriter {
        calls: usize,
        bytes: Vec<u8>,
        fail: bool,
    }
    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.fail {
                // Simulate delivery of a prefix followed by an I/O error.
                self.bytes.extend_from_slice(&bytes[..3]);
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected write failure",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            panic!("write_json must not flush")
        }
    }

    #[test]
    fn json_envelope_serialization_failure_writes_nothing() {
        let mut writer = RecordingWriter::default();
        let error = write_json(&mut writer, &outdated(FailsSerialization)).unwrap_err();
        assert!(matches!(error, OutputWriteError::Serialize(_)));
        assert!(error.source().is_some());
        assert_eq!(error.code(), codes::SERIALIZATION_FAILED);
        assert_eq!(writer.calls, 0);
        assert!(writer.bytes.is_empty());
    }

    #[test]
    fn json_envelope_writer_failure_is_not_retried() {
        let mut writer = RecordingWriter {
            fail: true,
            ..Default::default()
        };
        let error = write_json(&mut writer, &outdated(())).unwrap_err();
        assert!(
            matches!(&error, OutputWriteError::Write(e) if e.kind() == io::ErrorKind::BrokenPipe)
        );
        assert!(error.source().is_some());
        assert_eq!(error.code(), codes::IO_ERROR);
        assert_eq!(writer.calls, 1);
        assert_eq!(writer.bytes.len(), 3);
    }

    #[test]
    fn json_envelope_is_serialized_before_one_complete_write() {
        let mut writer = RecordingWriter::default();
        write_json(&mut writer, &outdated(())).unwrap();
        assert_eq!(writer.calls, 1);
        assert_eq!(writer.bytes.last(), Some(&b'\n'));
    }

    #[test]
    fn plan_json_dto_sorts_items_and_structured_owners_without_mutating_plan() {
        use crate::domain::agent::AgentKind;
        use crate::domain::link_plan::{LinkOwner, LinkPlan, LinkPlanItem, PlanAction as Action};
        use crate::domain::skill::{SkillName, SourcePath};
        use crate::domain::target::TargetPath;
        let dir = tempfile::tempdir().unwrap();
        let owners = [("zeta", "pi"), ("alpha", "universal"), ("alpha", "fx")]
            .into_iter()
            .map(|(skill, agent)| LinkOwner {
                skill: SkillName::new(skill).unwrap(),
                agent: agent.parse::<AgentKind>().unwrap(),
            })
            .collect::<Vec<_>>();
        let plan = LinkPlan::new(
            ["z", "a"]
                .into_iter()
                .map(|name| LinkPlanItem {
                    owners: owners.clone(),
                    source: SourcePath::new(dir.path().join("body")).unwrap(),
                    target: TargetPath::new(dir.path().join(name)).unwrap(),
                    action: Action::CreateSymlink,
                })
                .collect(),
        );
        let original = plan.clone();
        let value = serde_json::to_value(PlanData::from(&plan)).unwrap();
        assert_eq!(value["applicable"], true);
        assert_eq!(
            value["items"][0]["target"],
            dir.path().join("a").display().to_string()
        );
        assert_eq!(
            value["items"][1]["target"],
            dir.path().join("z").display().to_string()
        );
        assert_eq!(
            value["items"][0]["owners"],
            serde_json::json!([
                {"skill":"alpha","agent":"fx"},
                {"skill":"alpha","agent":"universal"},
                {"skill":"zeta","agent":"pi"}
            ])
        );
        assert_eq!(plan, original);
    }
    #[test]
    fn check_json_dto_preserves_all_variants_and_sorts_without_changing_report() {
        use crate::application::check::{CheckProblem as Problem, CheckReport};
        let report = CheckReport {
            problems: vec![
                Problem::TargetUnexpectedSymlink {
                    owners: vec![crate::domain::link_plan::LinkOwner {
                        skill: crate::domain::skill::SkillName::new("review").unwrap(),
                        agent: "pi".parse().unwrap(),
                    }],
                    skill: "review".into(),
                    agent: "pi".into(),
                    path: "target".into(),
                    actual_source: "other".into(),
                },
                Problem::TargetMissing {
                    owners: vec![crate::domain::link_plan::LinkOwner {
                        skill: crate::domain::skill::SkillName::new("zeta").unwrap(),
                        agent: "pi".parse().unwrap(),
                    }],
                    skill: "zeta".into(),
                    agent: "pi".into(),
                    path: "z".into(),
                },
                Problem::SourceHashDrift {
                    skill: "review".into(),
                    expected: "old-hash".into(),
                    actual: "new-hash".into(),
                },
                Problem::TargetConflict {
                    owners: vec![crate::domain::link_plan::LinkOwner {
                        skill: crate::domain::skill::SkillName::new("review").unwrap(),
                        agent: "pi".parse().unwrap(),
                    }],
                    skill: "review".into(),
                    agent: "pi".into(),
                    path: "target".into(),
                    reason: "regular file exists".into(),
                },
                Problem::InspectFailed {
                    owners: vec![crate::domain::link_plan::LinkOwner {
                        skill: crate::domain::skill::SkillName::new("review").unwrap(),
                        agent: "pi".parse().unwrap(),
                    }],
                    skill: "review".into(),
                    agent: "pi".into(),
                    message: "inspection error".into(),
                },
                Problem::IncludeMismatch {
                    skill: "review".into(),
                    expected: "SKILL.md".into(),
                    actual: "<full package>".into(),
                },
                Problem::HashFailed {
                    skill: "review".into(),
                    message: "hash error".into(),
                },
                Problem::BrokenSymlink {
                    owners: vec![crate::domain::link_plan::LinkOwner {
                        skill: crate::domain::skill::SkillName::new("review").unwrap(),
                        agent: "pi".parse().unwrap(),
                    }],
                    skill: "review".into(),
                    agent: "pi".into(),
                    path: "target".into(),
                    actual_source: "absent".into(),
                },
                Problem::TargetMissing {
                    owners: vec![crate::domain::link_plan::LinkOwner {
                        skill: crate::domain::skill::SkillName::new("alpha").unwrap(),
                        agent: "pi".parse().unwrap(),
                    }],
                    skill: "alpha".into(),
                    agent: "pi".into(),
                    path: "z".into(),
                },
                Problem::TargetMissing {
                    owners: vec![crate::domain::link_plan::LinkOwner {
                        skill: crate::domain::skill::SkillName::new("alpha").unwrap(),
                        agent: "fx".parse().unwrap(),
                    }],
                    skill: "alpha".into(),
                    agent: "fx".into(),
                    path: "z".into(),
                },
                Problem::TargetMissing {
                    owners: vec![crate::domain::link_plan::LinkOwner {
                        skill: crate::domain::skill::SkillName::new("alpha").unwrap(),
                        agent: "fx".parse().unwrap(),
                    }],
                    skill: "alpha".into(),
                    agent: "fx".into(),
                    path: "a".into(),
                },
            ],
        };
        let original = report.clone();
        let value = serde_json::to_value(CheckData::from(&report)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"healthy":false,"problems":[
                {"kind":"brokenSymlink","skill":"review","agent":"pi","path":"target","actualSource":"absent"},
                {"kind":"hashFailed","skill":"review","message":"hash error"},
                {"kind":"includeMismatch","skill":"review","expected":"SKILL.md","actual":"<full package>"},
                {"kind":"inspectFailed","skill":"review","agent":"pi","message":"inspection error"},
                {"kind":"sourceHashDrift","skill":"review","expected":"old-hash","actual":"new-hash"},
                {"kind":"targetConflict","skill":"review","agent":"pi","path":"target","reason":"regular file exists"},
                {"kind":"targetMissing","skill":"alpha","agent":"fx","path":"a"},
                {"kind":"targetMissing","skill":"alpha","agent":"fx","path":"z"},
                {"kind":"targetMissing","skill":"alpha","agent":"pi","path":"z"},
                {"kind":"targetMissing","skill":"zeta","agent":"pi","path":"z"},
                {"kind":"targetUnexpectedSymlink","skill":"review","agent":"pi","path":"target","actualSource":"other"}
            ]})
        );
        assert_eq!(report, original);
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/sksync-output.schema.json")).unwrap();
        for problem in value["problems"].as_array().unwrap() {
            let variant = schema["$defs"]["checkProblem"]["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .find(|variant| variant["properties"]["kind"]["const"] == problem["kind"])
                .unwrap();
            for field in variant["required"].as_array().unwrap() {
                assert!(
                    problem.get(field.as_str().unwrap()).is_some(),
                    "missing {field}"
                );
            }
            for (field, field_value) in problem.as_object().unwrap() {
                assert!(field_value.is_string());
                assert!(
                    variant["properties"].get(field).is_some(),
                    "unexpected {field}"
                );
            }
        }
        assert_eq!(
            serde_json::to_value(CheckData::from(&CheckReport { problems: vec![] })).unwrap(),
            serde_json::json!({"healthy":true,"problems":[]})
        );
    }
    #[test]
    fn check_json_shared_target_preserves_pairs_and_inspects_once_for_every_finding() {
        use crate::application::check::{check_lockfile_with_plan, group_check_problems};
        use crate::application::ports::{
            LinkStore, LinkStoreError, SourceHash, SourceHashStore, SourceHashStoreError,
            TargetState,
        };
        use crate::domain::agent::AgentKind;
        use crate::domain::link_plan::{LinkOwner, LinkPlan, LinkPlanItem, PlanAction};
        use crate::domain::lockfile::Lockfile;
        use crate::domain::skill::{SkillName, SourcePath};
        use crate::domain::target::TargetPath;
        use std::cell::Cell;

        struct NoHashes;
        impl SourceHashStore for NoHashes {
            fn hash_source(&self, _: &SourcePath) -> Result<SourceHash, SourceHashStoreError> {
                panic!("empty lockfile must not hash a body")
            }
        }
        struct Links {
            state: Option<TargetState>,
            inspections: Cell<usize>,
        }
        impl LinkStore for Links {
            fn inspect_target(
                &self,
                target: &TargetPath,
                _: &SourcePath,
            ) -> Result<TargetState, LinkStoreError> {
                self.inspections.set(self.inspections.get() + 1);
                self.state.clone().ok_or_else(|| LinkStoreError::Inspect {
                    path: target.as_path().display().to_string(),
                    source: std::io::Error::other("injected inspection failure"),
                })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let source = dir.path().join("body");
        let other = dir.path().join("other");
        let owners = [("review", "pi"), ("qa", "universal"), ("review", "fx")]
            .into_iter()
            .map(|(skill, agent)| LinkOwner {
                skill: SkillName::new(skill).unwrap(),
                agent: agent.parse::<AgentKind>().unwrap(),
            })
            .collect::<Vec<_>>();
        let plan = LinkPlan::new(vec![LinkPlanItem {
            owners,
            source: SourcePath::new(source).unwrap(),
            target: TargetPath::new(&target).unwrap(),
            action: PlanAction::CreateSymlink,
        }]);
        let lock = Lockfile {
            generated_by: "test".into(),
            generated_at: "test".into(),
            root: dir.path().to_path_buf(),
            skills: Default::default(),
        };
        for (state, kind) in [
            (Some(TargetState::Missing), "targetMissing"),
            (
                Some(TargetState::SymlinkToUnexpectedSource {
                    actual_source: other.clone(),
                }),
                "targetUnexpectedSymlink",
            ),
            (
                Some(TargetState::BrokenSymlink {
                    actual_source: other.clone(),
                }),
                "brokenSymlink",
            ),
            (Some(TargetState::RegularFileConflict), "targetConflict"),
            (Some(TargetState::DirectoryConflict), "targetConflict"),
            (None, "inspectFailed"),
        ] {
            let links = Links {
                state,
                inspections: Cell::new(0),
            };
            let report = check_lockfile_with_plan(&lock, &plan, &NoHashes, &links);
            let original = report.clone();
            assert_eq!(report.problems.len(), 1, "one physical observation");
            assert_eq!(group_check_problems(&report.problems)[0].count, 1);
            assert!(report.display_lines()[0].contains("agent=fx, pi, universal"));
            let value = serde_json::to_value(CheckData::from(&report)).unwrap();
            let rows = value["problems"].as_array().unwrap();
            assert_eq!(
                rows.iter()
                    .map(|row| (
                        row["skill"].as_str().unwrap(),
                        row["agent"].as_str().unwrap()
                    ))
                    .collect::<Vec<_>>(),
                [("qa", "universal"), ("review", "fx"), ("review", "pi")]
            );
            for row in rows {
                assert_eq!(row["kind"], kind);
                if kind == "inspectFailed" {
                    assert!(row["message"]
                        .as_str()
                        .unwrap()
                        .contains("injected inspection failure"));
                } else {
                    assert_eq!(row["path"], target.display().to_string());
                }
                if matches!(kind, "brokenSymlink" | "targetUnexpectedSymlink") {
                    assert_eq!(row["actualSource"], other.display().to_string());
                }
                if kind == "targetConflict" {
                    assert!(row["reason"].is_string());
                }
            }
            assert_eq!(
                links.inspections.get(),
                1,
                "DTO conversion must not reinspect"
            );
            assert_eq!(report, original);
        }
    }
}
