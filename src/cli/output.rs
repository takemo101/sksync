use std::io::Write;

use serde::Serialize;
use thiserror::Error;

// P09–P12 consume these output-only APIs. Remove each narrow allowance as its
// command adapter lands; the application/domain reports remain presentation-free.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JsonCommand {
    List,
    #[allow(dead_code)] // P10
    Plan,
    #[allow(dead_code)] // P11
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
    // Consumed by the command adapters in P09–P12.
    #[allow(dead_code)]
    pub const LOCKFILE_NOT_FOUND: &str = "LOCKFILE_NOT_FOUND";
    pub const INVALID_LOCKFILE: &str = "INVALID_LOCKFILE";
    pub const TARGET_RESOLUTION_FAILED: &str = "TARGET_RESOLUTION_FAILED";
    pub const INSPECTION_FAILED: &str = "INSPECTION_FAILED";
    // Consumed by the command adapters in P09–P12.
    #[allow(dead_code)]
    pub const REMOTE_QUERY_FAILED: &str = "REMOTE_QUERY_FAILED";
    // Consumed by the command adapters in P09–P12.
    #[allow(dead_code)]
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
}
