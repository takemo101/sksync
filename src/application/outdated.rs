use thiserror::Error;

use super::config::{InstallSource, ResolvedConfig};
use crate::domain::lockfile::Lockfile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedReport {
    pub rows: Vec<OutdatedRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedRow {
    pub skill: String,
    pub current: String,
    pub wanted: String,
    pub latest: String,
    pub source: String,
    pub status: String,
}

#[derive(Debug, Error)]
pub enum RemoteRefError {
    #[error("{0}")]
    Query(String),
}

pub trait RemoteRefResolver {
    fn git_remote_rev(&self, repo: &str, reference: &str) -> Result<String, RemoteRefError>;
}

pub fn collect_outdated(
    config: &ResolvedConfig,
    lockfile: &Lockfile,
    resolver: &impl RemoteRefResolver,
) -> OutdatedReport {
    let rows = config
        .skills
        .iter()
        .filter_map(|skill| {
            let locked = lockfile.skills.get(&skill.name)?;
            match (&skill.install_source, &locked.install_source) {
                (Some(InstallSource::Git(config_git)), Some(InstallSource::Git(locked_git))) => {
                    let wanted_ref = config_git.reference.as_deref().unwrap_or("HEAD");
                    let latest = resolver
                        .git_remote_rev(&config_git.url, wanted_ref)
                        .unwrap_or_else(|error| format!("error: {error}"));
                    let current = locked_git
                        .reference
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned());
                    if latest == current {
                        None
                    } else {
                        Some(OutdatedRow {
                            skill: skill.name.as_str().to_owned(),
                            current,
                            wanted: wanted_ref.to_owned(),
                            latest,
                            source: config_git.url.clone(),
                            status: "outdated".to_owned(),
                        })
                    }
                }
                (
                    Some(InstallSource::Registry(config_registry)),
                    Some(InstallSource::Registry(locked_registry)),
                ) => Some(OutdatedRow {
                    skill: skill.name.as_str().to_owned(),
                    current: locked_registry
                        .reference
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    wanted: config_registry
                        .reference
                        .clone()
                        .unwrap_or_else(|| "latest".to_owned()),
                    latest: "unsupported".to_owned(),
                    source: format!(
                        "registry:{}/{}",
                        config_registry.registry, config_registry.package
                    ),
                    status: "registry-provider-missing".to_owned(),
                }),
                _ => None,
            }
        })
        .collect();

    OutdatedReport { rows }
}

#[cfg(test)]
mod tests {
    use super::{collect_outdated, RemoteRefError, RemoteRefResolver};
    use crate::application::config::{
        GitInstallSource, InstallSource, ResolvedConfig, ResolvedSkill,
    };
    use crate::domain::lockfile::{Digest, LockedSkill, Lockfile};
    use crate::domain::skill::{SkillName, SourcePath};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    struct FakeResolver;

    impl RemoteRefResolver for FakeResolver {
        fn git_remote_rev(&self, _repo: &str, _reference: &str) -> Result<String, RemoteRefError> {
            Ok("new".to_owned())
        }
    }

    #[test]
    fn reports_git_ref_drift() {
        let name = SkillName::new("review").unwrap();
        let source = InstallSource::Git(GitInstallSource {
            url: "https://example.com/repo.git".to_owned(),
            reference: Some("main".to_owned()),
            path: "skills/review".into(),
        });
        let config = ResolvedConfig {
            skill_dir: SourcePath::new(".sksync/skills").unwrap(),
            agents: BTreeMap::new(),
            skills: vec![ResolvedSkill {
                name: name.clone(),
                source: SourcePath::new(".sksync/skills/review").unwrap(),
                install_source: Some(source.clone()),
                agents: Vec::new(),
            }],
        };
        let lockfile = Lockfile {
            generated_by: "test".to_owned(),
            generated_at: "test".to_owned(),
            root: PathBuf::from("."),
            skills: BTreeMap::from([(
                name,
                LockedSkill {
                    source: SourcePath::new(".sksync/skills/review").unwrap(),
                    install_source: Some(InstallSource::Git(GitInstallSource {
                        url: "https://example.com/repo.git".to_owned(),
                        reference: Some("old".to_owned()),
                        path: "skills/review".into(),
                    })),
                    hash: Digest::new("sha256-test").unwrap(),
                    files: Vec::new(),
                    targets: Vec::new(),
                },
            )]),
        };

        let report = collect_outdated(&config, &lockfile, &FakeResolver);

        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].current, "old");
        assert_eq!(report.rows[0].latest, "new");
    }
}
