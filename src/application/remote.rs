//! Read-only remote dependency source availability diagnosis for `doctor --remote`.
//!
//! This module inspects each dependency's current config install source and reports
//! Git sources whose configured path can no longer be resolved at the configured ref.
//! It never mutates config, lockfile, installed bodies, or symlinks.

use super::config::ResolvedConfig;
use crate::domain::source::{GitInstallSource, InstallSource};

/// Result of probing a single Git install source for remote availability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteSourceStatus {
    /// The configured path exists at the configured remote ref.
    Available,
    /// The remote repository is reachable, but the configured path is absent at the ref.
    PathMissing,
    /// The remote repository could not be cloned/fetched (auth/network/bad ref).
    RepoUnreachable(String),
}

/// Read-only probe for whether a Git source path resolves at its configured ref.
///
/// Implementations must not mutate any sksync state; they may use temporary clones.
pub trait RemoteSourceChecker {
    fn check_git_source(&self, source: &GitInstallSource) -> RemoteSourceStatus;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteSourceProblemKind {
    PathMissing,
    RepoUnreachable(String),
}

/// A single reported remote source problem for one dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSourceProblem {
    pub skill: String,
    pub global: bool,
    pub url: String,
    pub reference: String,
    pub path: String,
    pub kind: RemoteSourceProblemKind,
}

impl RemoteSourceProblem {
    pub fn scope_label(&self) -> &'static str {
        if self.global {
            "global"
        } else {
            "project"
        }
    }

    pub fn headline(&self) -> &'static str {
        match self.kind {
            RemoteSourceProblemKind::PathMissing => "REMOTE SOURCE MISSING",
            RemoteSourceProblemKind::RepoUnreachable(_) => "REMOTE SOURCE UNREACHABLE",
        }
    }

    pub fn reason(&self) -> String {
        match &self.kind {
            RemoteSourceProblemKind::PathMissing => {
                format!(
                    "path does not exist at current remote ref {}",
                    self.reference
                )
            }
            RemoteSourceProblemKind::RepoUnreachable(message) => {
                format!("remote repository is not accessible: {message}")
            }
        }
    }

    /// Scope-correct repair suggestion. `--global` is included only for global scope.
    pub fn suggestion(&self) -> String {
        if self.global {
            format!("sksync remove {} --global", self.skill)
        } else {
            format!("sksync remove {}", self.skill)
        }
    }
}

/// Summary of a remote source diagnosis run over one config scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSourceReport {
    pub problems: Vec<RemoteSourceProblem>,
    /// Number of Git dependencies that were probed remotely.
    pub checked: usize,
    /// Number of local (non-remote) dependencies skipped as not applicable.
    pub skipped_local: usize,
}

/// Inspect each dependency's current config install source for remote availability.
///
/// Local dependencies are skipped as not applicable and never produce a problem.
pub fn collect_remote_source_problems(
    config: &ResolvedConfig,
    global: bool,
    checker: &impl RemoteSourceChecker,
) -> RemoteSourceReport {
    let mut problems = Vec::new();
    let mut checked = 0;
    let mut skipped_local = 0;

    for skill in &config.skills {
        match &skill.install_source {
            Some(InstallSource::Git(git)) => {
                checked += 1;
                let kind = match checker.check_git_source(git) {
                    RemoteSourceStatus::Available => continue,
                    RemoteSourceStatus::PathMissing => RemoteSourceProblemKind::PathMissing,
                    RemoteSourceStatus::RepoUnreachable(message) => {
                        RemoteSourceProblemKind::RepoUnreachable(message)
                    }
                };
                problems.push(RemoteSourceProblem {
                    skill: skill.name.as_str().to_owned(),
                    global,
                    url: git.url.clone(),
                    reference: git.wanted_ref().to_owned(),
                    path: git.path.display().to_string(),
                    kind,
                });
            }
            Some(InstallSource::Local(_)) | None => skipped_local += 1,
        }
    }

    RemoteSourceReport {
        problems,
        checked,
        skipped_local,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        collect_remote_source_problems, RemoteSourceChecker, RemoteSourceProblemKind,
        RemoteSourceStatus,
    };
    use crate::application::config::{ResolvedConfig, ResolvedSkill};
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::source::{GitInstallSource, InstallSource};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    struct StubChecker {
        status: RemoteSourceStatus,
        calls: RefCell<usize>,
    }

    impl StubChecker {
        fn new(status: RemoteSourceStatus) -> Self {
            Self {
                status,
                calls: RefCell::new(0),
            }
        }
    }

    impl RemoteSourceChecker for StubChecker {
        fn check_git_source(&self, _source: &GitInstallSource) -> RemoteSourceStatus {
            *self.calls.borrow_mut() += 1;
            self.status.clone()
        }
    }

    fn git_source() -> InstallSource {
        InstallSource::Git(GitInstallSource {
            url: "https://github.com/owner/repo.git".to_owned(),
            reference: None,
            path: "skills/productivity/caveman".into(),
        })
    }

    fn config_with(skills: Vec<ResolvedSkill>) -> ResolvedConfig {
        ResolvedConfig {
            skill_dir: SourcePath::new(".sksync/skills").unwrap(),
            agents: BTreeMap::new(),
            skills,
            default_agents: Vec::new(),
        }
    }

    fn skill(name: &str, source: Option<InstallSource>) -> ResolvedSkill {
        ResolvedSkill {
            name: SkillName::new(name).unwrap(),
            source: SourcePath::new(format!(".sksync/skills/{name}")).unwrap(),
            install_source: source,
            include: None,
            agents: Vec::new(),
        }
    }

    #[test]
    fn project_missing_path_reports_without_global_suggestion() {
        let config = config_with(vec![skill("caveman", Some(git_source()))]);
        let checker = StubChecker::new(RemoteSourceStatus::PathMissing);

        let report = collect_remote_source_problems(&config, false, &checker);

        assert_eq!(report.checked, 1);
        assert_eq!(report.problems.len(), 1);
        let problem = &report.problems[0];
        assert_eq!(problem.skill, "caveman");
        assert_eq!(problem.scope_label(), "project");
        assert_eq!(problem.kind, RemoteSourceProblemKind::PathMissing);
        assert_eq!(problem.reference, "HEAD");
        assert_eq!(problem.suggestion(), "sksync remove caveman");
        assert!(!problem.suggestion().contains("--global"));
        assert_eq!(
            problem.reason(),
            "path does not exist at current remote ref HEAD"
        );
        assert_eq!(problem.headline(), "REMOTE SOURCE MISSING");
    }

    #[test]
    fn global_missing_path_includes_global_suggestion() {
        let config = config_with(vec![skill("caveman", Some(git_source()))]);
        let checker = StubChecker::new(RemoteSourceStatus::PathMissing);

        let report = collect_remote_source_problems(&config, true, &checker);

        let problem = &report.problems[0];
        assert_eq!(problem.scope_label(), "global");
        assert_eq!(problem.suggestion(), "sksync remove caveman --global");
    }

    #[test]
    fn available_source_reports_no_problem() {
        let config = config_with(vec![skill("review", Some(git_source()))]);
        let checker = StubChecker::new(RemoteSourceStatus::Available);

        let report = collect_remote_source_problems(&config, false, &checker);

        assert!(report.problems.is_empty());
        assert_eq!(report.checked, 1);
    }

    #[test]
    fn local_sources_are_skipped_and_never_probed() {
        let config = config_with(vec![
            skill(
                "local",
                Some(InstallSource::Local(PathBuf::from("./vendor/local"))),
            ),
            skill("legacy", None),
        ]);
        let checker = StubChecker::new(RemoteSourceStatus::PathMissing);

        let report = collect_remote_source_problems(&config, false, &checker);

        assert!(report.problems.is_empty());
        assert_eq!(report.checked, 0);
        assert_eq!(report.skipped_local, 2);
        assert_eq!(*checker.calls.borrow(), 0);
    }

    #[test]
    fn unreachable_repo_is_reported_separately_from_missing_path() {
        let config = config_with(vec![skill("review", Some(git_source()))]);
        let checker = StubChecker::new(RemoteSourceStatus::RepoUnreachable(
            "network down".to_owned(),
        ));

        let report = collect_remote_source_problems(&config, false, &checker);

        let problem = &report.problems[0];
        assert_eq!(problem.headline(), "REMOTE SOURCE UNREACHABLE");
        assert!(problem.reason().contains("network down"));
        assert!(matches!(
            problem.kind,
            RemoteSourceProblemKind::RepoUnreachable(_)
        ));
    }
}
