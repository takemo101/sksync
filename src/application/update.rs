use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use thiserror::Error;

use super::config::ResolvedConfig;
use super::ports::{
    CleanupWarning, LockfileStoreError, PreparedLockfileStore, PreparedSkill,
    PreparedSkillInstaller, SkillInstallError, SkillInstallRequest, SkillInstaller,
    SkillRollbackFailure,
};
use crate::domain::lockfile::{LockedSkill, Lockfile};
use crate::domain::skill::{SkillName, SkillNameError};
use crate::domain::source::InstallSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    pub updated: Vec<UpdatedSkill>,
    pub skipped: Vec<String>,
    pub warnings: Vec<CleanupWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatedSkill {
    pub name: String,
    pub source: String,
    pub resolved_source: InstallSource,
    pub destination: PathBuf,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("invalid skill name '{name}': {source}; use dependency keys from the chosen config")]
    InvalidSkillName {
        name: String,
        #[source]
        source: SkillNameError,
    },
    #[error("unknown skill '{skill}'; choose a dependency key from the chosen config")]
    UnknownSkill { skill: SkillName },
    #[error("skill '{skill}' has no dependency install source; configure a dependency before updating it")]
    NotDependency { skill: SkillName },
    #[error("selected update requires an existing readable lockfile; run a full install/update to establish the baseline")]
    MissingBaseline,
    #[error("lockfile has no entry for unselected skill '{skill}'; run a full install/update to establish the baseline")]
    MissingUnselectedEntry { skill: SkillName },
    #[error(transparent)]
    Install(#[from] SkillInstallError),
    #[error(transparent)]
    Lockfile(#[from] LockfileStoreError),
    #[error("failed to build update lockfile: {message}")]
    BuildLockfile { message: String },
    #[error("{original}; rollback failed: {details}", details = rollback_failure_details(.failures))]
    RollbackFailed {
        #[source]
        original: Box<UpdateError>,
        failures: Vec<SkillRollbackFailure>,
    },
}

/// No names means a full update; explicit names are dependency keys, not a smaller config.
#[allow(dead_code)] // P13 foundation; remove when MIK-018 connects positional CLI names.
pub fn resolve_update_selection(
    config: &ResolvedConfig,
    names: &[String],
) -> Result<Option<BTreeSet<SkillName>>, UpdateError> {
    if names.is_empty() {
        return Ok(None);
    }
    let mut selected = BTreeSet::new();
    for raw in names {
        let name = SkillName::new(raw).map_err(|source| UpdateError::InvalidSkillName {
            name: raw.clone(),
            source,
        })?;
        let skill = config
            .skills
            .iter()
            .find(|skill| skill.name == name)
            .ok_or_else(|| UpdateError::UnknownSkill {
                skill: name.clone(),
            })?;
        if skill.install_source.is_none() {
            return Err(UpdateError::NotDependency { skill: name });
        }
        selected.insert(name);
    }
    Ok(Some(selected))
}

/// The caller loads a readable baseline in the chosen scope, then validates it under guards.
#[allow(dead_code)] // P13 foundation; remove when MIK-018 connects the guarded baseline.
pub fn validate_selected_baseline(
    config: &ResolvedConfig,
    selected: &BTreeSet<SkillName>,
    previous: Option<&Lockfile>,
) -> Result<(), UpdateError> {
    let previous = previous.ok_or(UpdateError::MissingBaseline)?;
    for skill in &config.skills {
        if !selected.contains(&skill.name) && !previous.skills.contains_key(&skill.name) {
            return Err(UpdateError::MissingUnselectedEntry {
                skill: skill.name.clone(),
            });
        }
    }
    Ok(())
}

/// Preserve all other entries and metadata; the caller supplies successful generation metadata.
#[allow(dead_code)] // P13 foundation; remove when MIK-018 builds selected batch candidates.
pub fn merge_selected_lockfile(
    previous: &Lockfile,
    replacements: BTreeMap<SkillName, LockedSkill>,
) -> Lockfile {
    let mut merged = previous.clone();
    merged.skills.extend(replacements);
    merged
}

fn rollback_failure_details(failures: &[SkillRollbackFailure]) -> String {
    failures
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// The caller holds writer guards through preparation, rollback and finalization.
/// A successful lockfile publication is the only commit point.
pub fn update_dependency_batch<I, L, F>(
    config: &ResolvedConfig,
    only: Option<&BTreeSet<SkillName>>,
    installer: &I,
    store: &L,
    build_lockfile: F,
) -> Result<UpdateReport, UpdateError>
where
    I: PreparedSkillInstaller,
    L: PreparedLockfileStore,
    F: FnOnce(&[PreparedSkill<I::Receipt>]) -> Result<Lockfile, UpdateError>,
{
    let mut report = UpdateReport {
        updated: Vec::new(),
        skipped: Vec::new(),
        warnings: Vec::new(),
    };
    let mut prepared = Vec::new();
    let result = (|| -> Result<(), UpdateError> {
        let mut skills = config
            .skills
            .iter()
            .filter(|skill| only.is_none_or(|names| names.contains(&skill.name)))
            .collect::<Vec<_>>();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        for skill in skills {
            let Some(source) = &skill.install_source else {
                report.skipped.push(skill.name.as_str().to_owned());
                continue;
            };
            let request = SkillInstallRequest {
                managed_root: config.skill_dir.as_path().to_path_buf(),
                source: source.clone(),
                include: skill.include.clone(),
            };
            prepared.push(installer.prepare_skill(
                &request,
                skill.source.as_path(),
                skill.name.as_str(),
            )?);
        }
        if prepared.is_empty() {
            return Ok(());
        }
        let candidate = build_lockfile(&prepared)?;
        let serialized = store.prepare_lockfile(&candidate)?;
        for skill in &mut prepared {
            installer.publish_skill(&mut skill.receipt)?;
        }
        report.warnings.extend(store.publish_lockfile(&serialized)?);
        Ok(())
    })();
    if let Err(original) = result {
        let failures = prepared
            .iter_mut()
            .rev()
            .filter_map(|skill| installer.rollback_skill(&mut skill.receipt).err())
            .collect::<Vec<_>>();
        return if failures.is_empty() {
            Err(original)
        } else {
            Err(UpdateError::RollbackFailed {
                original: Box::new(original),
                failures,
            })
        };
    }
    for mut skill in prepared {
        report.warnings.extend(skill.installed.warnings);
        report
            .warnings
            .extend(installer.finalize_skill(&mut skill.receipt));
        report.updated.push(UpdatedSkill {
            name: skill.name.as_str().to_owned(),
            source: skill.installed.label,
            resolved_source: skill.installed.resolved_source,
            destination: skill.destination,
        });
    }
    Ok(report)
}

pub fn update_dependencies(
    config: &ResolvedConfig,
    installer: &impl SkillInstaller,
) -> Result<UpdateReport, UpdateError> {
    update_selected_dependencies(config, installer, None)
}

/// Install/fetch dependencies, optionally restricted to a subset of skill names.
///
/// `add` uses the `only` filter so that adding a new dependency never refetches,
/// updates, or rewrites unrelated existing dependencies. Passing `None` installs
/// every dependency for immediate installer callers; `update` uses the batch seam.
pub fn update_selected_dependencies(
    config: &ResolvedConfig,
    installer: &impl SkillInstaller,
    only: Option<&BTreeSet<String>>,
) -> Result<UpdateReport, UpdateError> {
    let mut report = UpdateReport {
        updated: Vec::new(),
        skipped: Vec::new(),
        warnings: Vec::new(),
    };

    for skill in &config.skills {
        if let Some(only) = only {
            if !only.contains(skill.name.as_str()) {
                continue;
            }
        }
        let Some(install_source) = &skill.install_source else {
            report.skipped.push(skill.name.as_str().to_owned());
            continue;
        };
        let destination = skill.source.as_path().to_path_buf();
        let request = SkillInstallRequest {
            managed_root: config.skill_dir.as_path().to_path_buf(),
            source: install_source.clone(),
            include: skill.include.clone(),
        };
        let installed = installer.install_skill(&request, &destination, skill.name.as_str())?;
        report.warnings.extend(installed.warnings);
        report.updated.push(UpdatedSkill {
            name: skill.name.as_str().to_owned(),
            source: installed.label,
            resolved_source: installed.resolved_source,
            destination,
        });
    }

    Ok(report)
}

pub fn apply_update_report_sources(config: &mut ResolvedConfig, report: &UpdateReport) {
    for updated in &report.updated {
        if let Some(skill) = config
            .skills
            .iter_mut()
            .find(|skill| skill.name.as_str() == updated.name)
        {
            skill.install_source = Some(updated.resolved_source.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::update_dependencies;
    use crate::application::config::{ResolvedAgent, ResolvedConfig, ResolvedSkill};
    use crate::application::ports::{
        InstalledSkillSource, SkillInstallError, SkillInstallRequest, SkillInstaller,
    };
    use crate::domain::agent::AgentKind;
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::source::InstallSource;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    struct FakeInstaller {
        installed: RefCell<Vec<PathBuf>>,
    }

    impl SkillInstaller for FakeInstaller {
        fn install_skill(
            &self,
            request: &SkillInstallRequest,
            destination: &Path,
            _skill_name: &str,
        ) -> Result<InstalledSkillSource, SkillInstallError> {
            self.installed.borrow_mut().push(destination.to_path_buf());
            Ok(InstalledSkillSource {
                label: format!("{:?}", request.source),
                resolved_source: request.source.clone(),
                warnings: Vec::new(),
            })
        }
    }

    #[test]
    fn dependency_is_installed_into_skill_dir() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("skills");
        let config = config(
            skill_dir.clone(),
            InstallSource::Local(temp.path().join("remote/review")),
        );
        let installer = FakeInstaller {
            installed: RefCell::new(Vec::new()),
        };

        let report = update_dependencies(&config, &installer).unwrap();

        assert_eq!(report.updated.len(), 1);
        assert_eq!(report.updated[0].name, "review");
        assert_eq!(installer.installed.borrow()[0], skill_dir.join("review"));
    }

    #[test]
    fn update_passes_configured_managed_root_and_collects_cleanup_warnings() {
        struct WarningInstaller;
        impl SkillInstaller for WarningInstaller {
            fn install_skill(
                &self,
                request: &SkillInstallRequest,
                destination: &Path,
                _name: &str,
            ) -> Result<InstalledSkillSource, SkillInstallError> {
                assert_eq!(
                    request.managed_root,
                    destination.parent().unwrap().parent().unwrap()
                );
                Ok(InstalledSkillSource {
                    label: "local".into(),
                    resolved_source: request.source.clone(),
                    warnings: vec![crate::application::ports::CleanupWarning {
                        path: request.managed_root.join("retained"),
                        message: "cleanup failed".into(),
                    }],
                })
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let mut config = config(
            temp.path().join("skills"),
            InstallSource::Local(temp.path().join("local")),
        );
        config.skills[0].source =
            SourcePath::new(config.skill_dir.as_path().join("namespace/review")).unwrap();
        let report = update_dependencies(&config, &WarningInstaller).unwrap();
        assert_eq!(report.updated.len(), 1);
        assert_eq!(
            report.warnings,
            vec![crate::application::ports::CleanupWarning {
                path: config.skill_dir.as_path().join("retained"),
                message: "cleanup failed".into()
            }]
        );
    }

    pub(super) fn config(skill_dir: PathBuf, install_source: InstallSource) -> ResolvedConfig {
        let mut agents = BTreeMap::new();
        agents.insert(
            "pi".to_owned(),
            ResolvedAgent {
                kind: AgentKind::Pi,
                enabled: true,
                scope: Scope::User,
                target_dir: None,
            },
        );
        ResolvedConfig {
            skill_dir: SourcePath::new(skill_dir.clone()).unwrap(),
            agents,
            skills: vec![ResolvedSkill {
                name: SkillName::new("review").unwrap(),
                source: SourcePath::new(skill_dir.join("review")).unwrap(),
                install_source: Some(install_source),
                include: None,
                agents: vec![AgentKind::Pi],
            }],
            default_agents: Vec::new(),
        }
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::application::ports::{
        InstalledSkillSource, LockfileStore, LockfileStoreError, PreparedLockfileStore,
        PreparedSkill, PreparedSkillInstaller, SkillRollbackFailure,
    };
    use crate::domain::lockfile::{Digest, Lockfile};
    use crate::domain::skill::{SkillName, SourcePath};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    #[derive(Default)]
    struct BatchFaults {
        prepare: Option<String>,
        publish: Option<String>,
        serialize: bool,
        lock_publish: bool,
        rollback: bool,
        cleanup: bool,
    }

    struct BatchAdapter {
        root: PathBuf,
        faults: BatchFaults,
        events: RefCell<Vec<String>>,
    }

    struct Receipt {
        name: SkillName,
        path: PathBuf,
        old: Option<Vec<u8>>,
        backup: PathBuf,
        published: bool,
    }

    impl PreparedSkillInstaller for BatchAdapter {
        type Receipt = Receipt;
        fn prepare_skill(
            &self,
            request: &SkillInstallRequest,
            destination: &Path,
            name: &str,
        ) -> Result<PreparedSkill<Receipt>, SkillInstallError> {
            assert_eq!(request.managed_root, self.root.join("skills"));
            self.events.borrow_mut().push(format!("prepare:{name}"));
            if self.faults.prepare.as_deref() == Some(name) {
                return Err(SkillInstallError::Prepare {
                    path: destination.display().to_string(),
                    message: "prepare failure".into(),
                });
            }
            let name = SkillName::new(name).unwrap();
            Ok(PreparedSkill {
                name: name.clone(),
                destination: destination.into(),
                installed: InstalledSkillSource {
                    label: "new source".into(),
                    resolved_source: request.source.clone(),
                    warnings: Vec::new(),
                },
                hash: Digest::new("new-hash").unwrap(),
                files: Vec::new(),
                receipt: Receipt {
                    name: name.clone(),
                    path: destination.into(),
                    old: fs::read(destination).ok(),
                    backup: self.root.join(format!("backup-{name}")),
                    published: false,
                },
            })
        }
        fn publish_skill(&self, receipt: &mut Receipt) -> Result<(), SkillInstallError> {
            self.events
                .borrow_mut()
                .push(format!("publish:{}", receipt.name));
            if let Some(old) = &receipt.old {
                fs::write(&receipt.backup, old).unwrap();
            }
            fs::write(&receipt.path, b"new body").unwrap();
            receipt.published = true;
            if self.faults.publish.as_deref() == Some(receipt.name.as_str()) {
                return Err(SkillInstallError::Prepare {
                    path: receipt.path.display().to_string(),
                    message: "publication failure".into(),
                });
            }
            Ok(())
        }
        fn rollback_skill(&self, receipt: &mut Receipt) -> Result<(), SkillRollbackFailure> {
            self.events
                .borrow_mut()
                .push(format!("rollback:{}", receipt.name));
            if self.faults.rollback && receipt.published {
                return Err(SkillRollbackFailure {
                    skill: receipt.name.clone(),
                    retained_backup: receipt.old.as_ref().map(|_| receipt.backup.clone()),
                    message: "restore failure".into(),
                });
            }
            if receipt.published {
                match &receipt.old {
                    Some(old) => fs::write(&receipt.path, old).unwrap(),
                    None => fs::remove_file(&receipt.path).unwrap(),
                }
                if receipt.old.is_some() {
                    fs::remove_file(&receipt.backup).unwrap();
                }
            }
            Ok(())
        }
        fn finalize_skill(&self, receipt: &mut Receipt) -> Vec<CleanupWarning> {
            self.events
                .borrow_mut()
                .push(format!("finalize:{}", receipt.name));
            if self.faults.cleanup {
                return vec![CleanupWarning {
                    path: receipt.backup.clone(),
                    message: "cleanup failure".into(),
                }];
            }
            if receipt.old.is_some() {
                fs::remove_file(&receipt.backup).unwrap();
            }
            Vec::new()
        }
    }

    impl LockfileStore for BatchAdapter {
        fn write(&self, _: &Lockfile) -> Result<(), LockfileStoreError> {
            panic!("batch must use prepared lockfile seam")
        }
    }
    impl PreparedLockfileStore for BatchAdapter {
        type Prepared = Vec<u8>;
        fn prepare_lockfile(&self, _: &Lockfile) -> Result<Vec<u8>, LockfileStoreError> {
            self.events.borrow_mut().push("serialize".into());
            if self.faults.serialize {
                return Err(LockfileStoreError::Write("serialization failure".into()));
            }
            Ok(b"new lock bytes\n".to_vec())
        }
        fn publish_lockfile(
            &self,
            bytes: &Vec<u8>,
        ) -> Result<Vec<CleanupWarning>, LockfileStoreError> {
            self.events.borrow_mut().push("commit".into());
            if self.faults.lock_publish {
                return Err(LockfileStoreError::Write("lock publication failure".into()));
            }
            fs::write(self.root.join("lock"), bytes).unwrap();
            Ok(vec![CleanupWarning {
                path: self.root.join("lock-temp"),
                message: "lock cleanup warning".into(),
            }])
        }
    }

    fn fixture(root: &Path, faults: BatchFaults, absent: bool) -> (ResolvedConfig, BatchAdapter) {
        let skill_dir = root.join("skills");
        fs::create_dir(&skill_dir).unwrap();
        let mut config = super::tests::config(
            skill_dir.clone(),
            InstallSource::Local(root.join("source-alpha")),
        );
        config.skills[0].name = SkillName::new("alpha").unwrap();
        config.skills[0].source = SourcePath::new(skill_dir.join("alpha")).unwrap();
        let mut beta = config.skills[0].clone();
        beta.name = SkillName::new("beta").unwrap();
        beta.source = SourcePath::new(skill_dir.join("beta")).unwrap();
        config.skills.push(beta);
        fs::write(skill_dir.join("alpha"), b"old alpha").unwrap();
        if !absent {
            fs::write(skill_dir.join("beta"), b"old beta").unwrap();
            fs::write(root.join("lock"), b"old raw lock\n").unwrap();
        }
        (
            config,
            BatchAdapter {
                root: root.into(),
                faults,
                events: RefCell::new(Vec::new()),
            },
        )
    }
    fn candidate(_: &[PreparedSkill<Receipt>]) -> Result<Lockfile, UpdateError> {
        Ok(Lockfile {
            generated_by: "test".into(),
            generated_at: "test".into(),
            root: PathBuf::from("."),
            skills: BTreeMap::new(),
        })
    }

    #[test]
    fn update_batch_precommit_failures_restore_all_bodies_and_raw_lock_or_absence() {
        for absent in [false, true] {
            for phase in [
                "prepare",
                "build",
                "serialize",
                "publish-alpha",
                "publish-beta",
                "lock",
            ] {
                let dir = tempfile::tempdir().unwrap();
                let faults = BatchFaults {
                    prepare: (phase == "prepare").then(|| "beta".into()),
                    publish: phase.strip_prefix("publish-").map(str::to_owned),
                    serialize: phase == "serialize",
                    lock_publish: phase == "lock",
                    ..Default::default()
                };
                let (config, adapter) = fixture(dir.path(), faults, absent);
                let original_lock = fs::read(dir.path().join("lock")).ok();
                let result =
                    update_dependency_batch(&config, None, &adapter, &adapter, |prepared| {
                        if phase == "build" {
                            Err(UpdateError::BuildLockfile {
                                message: "candidate failure".into(),
                            })
                        } else {
                            candidate(prepared)
                        }
                    });
                assert!(result.is_err(), "{phase}");
                assert_eq!(
                    fs::read(dir.path().join("skills/alpha")).unwrap(),
                    b"old alpha",
                    "{phase}"
                );
                assert_eq!(
                    fs::read(dir.path().join("skills/beta")).ok(),
                    (!absent).then(|| b"old beta".to_vec()),
                    "{phase}"
                );
                assert_eq!(
                    fs::read(dir.path().join("lock")).ok(),
                    original_lock,
                    "{phase}"
                );
                let events = adapter.events.borrow();
                assert!(!events.iter().any(|event| event.starts_with("finalize:")));
                let rollback: Vec<_> = events
                    .iter()
                    .filter(|event| event.starts_with("rollback:"))
                    .map(String::as_str)
                    .collect();
                assert_eq!(
                    rollback,
                    if phase == "prepare" {
                        vec!["rollback:alpha"]
                    } else {
                        vec!["rollback:beta", "rollback:alpha"]
                    }
                );
                if ["prepare", "build", "serialize"].contains(&phase) {
                    assert!(!events.iter().any(|event| event.starts_with("publish:")));
                }
            }
        }
    }

    #[test]
    fn update_batch_commits_last_then_returns_cleanup_warnings_without_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let (config, adapter) = fixture(
            dir.path(),
            BatchFaults {
                cleanup: true,
                ..Default::default()
            },
            false,
        );
        let report = update_dependency_batch(&config, None, &adapter, &adapter, candidate).unwrap();
        assert_eq!(report.updated.len(), 2);
        assert_eq!(report.warnings.len(), 3);
        assert_eq!(
            fs::read(dir.path().join("lock")).unwrap(),
            b"new lock bytes\n"
        );
        for name in ["alpha", "beta"] {
            assert_eq!(
                fs::read(dir.path().join("skills").join(name)).unwrap(),
                b"new body"
            );
            assert!(dir.path().join(format!("backup-{name}")).exists());
        }
        assert_eq!(
            *adapter.events.borrow(),
            [
                "prepare:alpha",
                "prepare:beta",
                "serialize",
                "publish:alpha",
                "publish:beta",
                "commit",
                "finalize:alpha",
                "finalize:beta"
            ]
        );
    }

    #[test]
    fn update_batch_aggregates_rollback_failures_and_displays_original_and_recovery_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (config, adapter) = fixture(
            dir.path(),
            BatchFaults {
                rollback: true,
                lock_publish: true,
                ..Default::default()
            },
            false,
        );
        let error =
            update_dependency_batch(&config, None, &adapter, &adapter, candidate).unwrap_err();
        let message = error.to_string();
        let UpdateError::RollbackFailed { original, failures } = error else {
            panic!("missing rollback failures")
        };
        assert!(matches!(*original, UpdateError::Lockfile(_)));
        assert_eq!(failures.len(), 2);
        assert!(message.contains("lock publication failure"));
        for name in ["alpha", "beta"] {
            let backup = dir.path().join(format!("backup-{name}"));
            assert!(message.contains(backup.to_str().unwrap()));
            assert_eq!(fs::read(backup).unwrap(), format!("old {name}").as_bytes());
        }
        assert_eq!(
            fs::read(dir.path().join("lock")).unwrap(),
            b"old raw lock\n"
        );
        assert!(!adapter
            .events
            .borrow()
            .iter()
            .any(|event| event.starts_with("finalize:")));
    }

    #[test]
    fn update_batch_empty_selection_never_builds_or_writes_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, adapter) = fixture(dir.path(), BatchFaults::default(), false);
        for skill in &mut config.skills {
            skill.install_source = None;
        }
        let report = update_dependency_batch(&config, None, &adapter, &adapter, |_| {
            panic!("empty selection must not build")
        })
        .unwrap();
        assert!(report.updated.is_empty());
        assert_eq!(report.skipped, ["alpha", "beta"]);
        assert!(adapter.events.borrow().is_empty());
        assert_eq!(
            fs::read(dir.path().join("lock")).unwrap(),
            b"old raw lock\n"
        );
    }

    #[test]
    fn update_batch_name_filter_does_not_prepare_unselected_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let (config, adapter) = fixture(dir.path(), BatchFaults::default(), false);
        let only = BTreeSet::from([SkillName::new("alpha").unwrap()]);
        let report =
            update_dependency_batch(&config, Some(&only), &adapter, &adapter, |prepared| {
                assert_eq!(prepared.len(), 1);
                assert_eq!(prepared[0].name.as_str(), "alpha");
                candidate(prepared)
            })
            .unwrap();
        assert_eq!(report.updated.len(), 1);
        assert_eq!(
            fs::read(dir.path().join("skills/beta")).unwrap(),
            b"old beta"
        );
        assert!(!adapter
            .events
            .borrow()
            .iter()
            .any(|event| event.ends_with(":beta")));
    }
}

#[cfg(test)]
mod selected_update_tests {
    use super::*;
    use crate::domain::agent::AgentKind;
    use crate::domain::lockfile::{Digest, LinkType, LockedFile, LockedSkill, LockedTarget};
    use crate::domain::package_filter::PackageFilter;
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillNameError, SourcePath};
    use crate::domain::source::GitInstallSource;
    use crate::domain::target::TargetPath;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn config(root: &Path) -> ResolvedConfig {
        let mut config = super::tests::config(
            root.join("skills"),
            InstallSource::Local(root.join("local/review")),
        );
        let mut qa = config.skills[0].clone();
        qa.name = SkillName::new("qa").unwrap();
        qa.source = SourcePath::new(root.join("skills/qa")).unwrap();
        qa.install_source = Some(InstallSource::Git(GitInstallSource {
            url: root.join("local-repo.git").to_string_lossy().into_owned(),
            reference: Some("main".into()),
            path: "skills/qa".into(),
        }));
        qa.include = Some(PackageFilter::manifest_only());
        config.skills.push(qa);
        config
    }

    fn locked(config: &ResolvedConfig) -> Lockfile {
        Lockfile {
            generated_by: "old generator".into(),
            generated_at: "old timestamp".into(),
            root: config.skill_dir.as_path().parent().unwrap().into(),
            skills: config
                .skills
                .iter()
                .map(|skill| {
                    (
                        skill.name.clone(),
                        LockedSkill {
                            source: skill.source.clone(),
                            install_source: skill.install_source.clone(),
                            include: skill.include.clone(),
                            hash: Digest::new(format!("old-{}", skill.name)).unwrap(),
                            files: vec![LockedFile {
                                path: "SKILL.md".into(),
                                hash: Digest::new("old-file").unwrap(),
                            }],
                            targets: vec![LockedTarget {
                                agent: AgentKind::Pi,
                                scope: Scope::Project,
                                path: TargetPath::new(
                                    config.skill_dir.as_path().join("legacy-target"),
                                )
                                .unwrap(),
                                link_type: LinkType::Symlink,
                            }],
                        },
                    )
                })
                .collect(),
        }
    }

    fn select(config: &ResolvedConfig, names: &[&str]) -> Result<BTreeSet<SkillName>, UpdateError> {
        resolve_update_selection(
            config,
            &names.iter().map(|name| (*name).into()).collect::<Vec<_>>(),
        )
        .map(|selection| selection.expect("explicit names"))
    }

    #[test]
    fn selected_update_empty_input_means_full_even_without_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(dir.path());
        assert_eq!(resolve_update_selection(&config, &[]).unwrap(), None);
        for skill in &mut config.skills {
            skill.install_source = None;
        }
        assert_eq!(resolve_update_selection(&config, &[]).unwrap(), None);
        config.skills.clear();
        assert_eq!(resolve_update_selection(&config, &[]).unwrap(), None);
    }

    #[test]
    fn selected_update_trims_deduplicates_and_sorts_local_and_git_keys() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let snapshot = config.clone();
        let selection = select(&config, &[" review ", "qa", "review", " qa "]).unwrap();
        assert_eq!(
            selection.iter().map(SkillName::as_str).collect::<Vec<_>>(),
            ["qa", "review"]
        );
        assert_eq!(config, snapshot);
    }

    #[test]
    fn selected_update_matches_case_sensitively() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(dir.path());
        assert!(
            matches!(select(&config, &["Review"]), Err(UpdateError::UnknownSkill { skill }) if skill.as_str() == "Review")
        );
        let mut upper = config.skills[0].clone();
        upper.name = SkillName::new("Review").unwrap();
        config.skills.push(upper);
        assert_eq!(select(&config, &["Review", "review"]).unwrap().len(), 2);
    }

    #[test]
    fn selected_update_rejects_invalid_names_with_typed_causes() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        for (name, expected) in [
            (".", SkillNameError::ReservedPathComponent),
            (" .. ", SkillNameError::ReservedPathComponent),
            (" ", SkillNameError::Empty),
            ("skills/review", SkillNameError::ContainsPathSeparator),
            ("skills\\review", SkillNameError::ContainsPathSeparator),
        ] {
            let error = select(&config, &["review", name]).unwrap_err();
            assert!(error.to_string().contains("skill name"));
            assert!(
                matches!(error, UpdateError::InvalidSkillName { name: raw, source } if raw == name && source == expected)
            );
        }
    }

    #[test]
    fn selected_update_rejects_whole_set_for_unknown_or_legacy_only_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(dir.path());
        assert!(
            matches!(select(&config, &["review", " missing "]), Err(UpdateError::UnknownSkill { skill }) if skill.as_str() == "missing")
        );
        config.skills[1].install_source = None;
        let error = select(&config, &["review", "qa"]).unwrap_err();
        assert!(error.to_string().contains("dependency"));
        assert!(matches!(error, UpdateError::NotDependency { skill } if skill.as_str() == "qa"));
    }

    #[test]
    fn selected_update_requires_baseline_even_when_every_skill_is_selected() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let selected = select(&config, &["review", "qa"]).unwrap();
        let error = validate_selected_baseline(&config, &selected, None).unwrap_err();
        assert!(error.to_string().contains("full install/update"));
        assert!(matches!(error, UpdateError::MissingBaseline));
    }

    #[test]
    fn selected_update_requires_unselected_dependency_and_legacy_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(dir.path());
        let selected = select(&config, &["review"]).unwrap();
        let mut previous = locked(&config);
        previous.skills.remove(&SkillName::new("qa").unwrap());
        for legacy in [false, true] {
            if legacy {
                config.skills[1].install_source = None;
            }
            let error =
                validate_selected_baseline(&config, &selected, Some(&previous)).unwrap_err();
            assert!(error.to_string().contains("full install/update"));
            assert!(
                matches!(error, UpdateError::MissingUnselectedEntry { skill } if skill.as_str() == "qa")
            );
        }
    }

    #[test]
    fn selected_update_allows_absent_selected_entry_and_does_not_inspect_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let selected = select(&config, &["review"]).unwrap();
        let mut previous = locked(&config);
        previous.skills.remove(&SkillName::new("review").unwrap());
        assert!(validate_selected_baseline(&config, &selected, Some(&previous)).is_ok());
        assert!(!config.skill_dir.as_path().exists());
        let all = select(&config, &["review", "qa"]).unwrap();
        previous.skills.clear();
        assert!(validate_selected_baseline(&config, &all, Some(&previous)).is_ok());
    }

    #[test]
    fn selected_update_merge_preserves_unselected_and_stale_entries_and_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let mut previous = locked(&config);
        let review = SkillName::new("review").unwrap();
        let qa = SkillName::new("qa").unwrap();
        let stale = SkillName::new("removed-dependency").unwrap();
        previous
            .skills
            .insert(stale.clone(), previous.skills[&qa].clone());
        let snapshot = previous.clone();
        let mut replacement = previous.skills[&review].clone();
        replacement.source = SourcePath::new(dir.path().join("skills/new-review")).unwrap();
        replacement.install_source = config.skills[1].install_source.clone();
        replacement.include = Some(PackageFilter::manifest_only());
        replacement.hash = Digest::new("new-body").unwrap();
        replacement.files[0].hash = Digest::new("new-file").unwrap();
        replacement.targets.clear();
        let merged = merge_selected_lockfile(
            &previous,
            BTreeMap::from([(review.clone(), replacement.clone())]),
        );
        let mut expected = snapshot.clone();
        expected.skills.insert(review, replacement);
        assert_eq!(merged, expected);
        assert_eq!(previous, snapshot);
        assert_eq!(
            merge_selected_lockfile(&previous, BTreeMap::new()),
            previous
        );
    }

    #[test]
    fn selected_update_merge_inserts_new_selected_entry() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let mut previous = locked(&config);
        let review = SkillName::new("review").unwrap();
        let replacement = previous.skills.remove(&review).unwrap();
        let merged = merge_selected_lockfile(
            &previous,
            BTreeMap::from([(review.clone(), replacement.clone())]),
        );
        assert_eq!(merged.skills[&review], replacement);
        assert_eq!(merged.skills.len(), previous.skills.len() + 1);
    }

    #[test]
    fn selected_update_merge_leaves_legacy_target_normalization_to_v5_serializer() {
        use crate::infrastructure::json::{read_lockfile, write_lockfile};
        for version in [2, 3, 4, 5] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("sksync-lock.json");
            let raw = serde_json::json!({
                "lockfileVersion": version, "generatedBy": "old", "generatedAt": "old", "root": ".",
                "skills": {"qa": {
                    "source": "skills/qa", "installSource": {"type": "local", "path": "local/qa"},
                    "include": ["SKILL.md"], "hash": "old-hash", "files": [{"path": "SKILL.md", "hash": "old-file"}],
                    "targets": [{"agent": "pi", "scope": "project", "path": ".pi/skills/qa", "linkType": "symlink"}]
                }}
            });
            std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
            let previous = read_lockfile(&path).unwrap();
            let config = config(dir.path());
            let replacement = locked(&config).skills[&SkillName::new("review").unwrap()].clone();
            let merged = merge_selected_lockfile(
                &previous,
                BTreeMap::from([(SkillName::new("review").unwrap(), replacement)]),
            );
            assert_eq!(
                merged.skills[&SkillName::new("qa").unwrap()],
                previous.skills[&SkillName::new("qa").unwrap()]
            );
            write_lockfile(&path, &merged).unwrap();
            let published: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(published["lockfileVersion"], 5);
            assert_eq!(published["root"], ".");
            let mut expected = raw["skills"]["qa"].clone();
            expected.as_object_mut().unwrap().remove("targets");
            assert_eq!(published["skills"]["qa"], expected);
            assert!(published["skills"]["review"].get("targets").is_none());
        }
    }
}
