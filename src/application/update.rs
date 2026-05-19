use std::path::PathBuf;

use thiserror::Error;

use super::config::InstallSource;
use super::config::ResolvedConfig;
use super::ports::{SkillInstallError, SkillInstaller};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    pub updated: Vec<UpdatedSkill>,
    pub skipped: Vec<String>,
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
    #[error(transparent)]
    Install(#[from] SkillInstallError),
}

pub fn update_dependencies(
    config: &ResolvedConfig,
    installer: &impl SkillInstaller,
) -> Result<UpdateReport, UpdateError> {
    let mut report = UpdateReport {
        updated: Vec::new(),
        skipped: Vec::new(),
    };

    for skill in &config.skills {
        let Some(install_source) = &skill.install_source else {
            report.skipped.push(skill.name.as_str().to_owned());
            continue;
        };
        let destination = skill.source.as_path().to_path_buf();
        let installed =
            installer.install_skill(install_source, &destination, skill.name.as_str())?;
        report.updated.push(UpdatedSkill {
            name: skill.name.as_str().to_owned(),
            source: installed.label,
            resolved_source: installed.resolved_source,
            destination,
        });
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::update_dependencies;
    use crate::application::config::{InstallSource, ResolvedAgent, ResolvedConfig, ResolvedSkill};
    use crate::application::ports::{InstalledSkillSource, SkillInstallError, SkillInstaller};
    use crate::domain::agent::AgentKind;
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    struct FakeInstaller {
        installed: RefCell<Vec<PathBuf>>,
    }

    impl SkillInstaller for FakeInstaller {
        fn install_skill(
            &self,
            source: &InstallSource,
            destination: &Path,
            _skill_name: &str,
        ) -> Result<InstalledSkillSource, SkillInstallError> {
            self.installed.borrow_mut().push(destination.to_path_buf());
            Ok(InstalledSkillSource {
                label: format!("{source:?}"),
                resolved_source: source.clone(),
            })
        }
    }

    #[test]
    fn dependency_is_installed_into_skill_dir() {
        let skill_dir = PathBuf::from("skills");
        let config = config(
            skill_dir.clone(),
            InstallSource::Local(PathBuf::from("remote/review")),
        );
        let installer = FakeInstaller {
            installed: RefCell::new(Vec::new()),
        };

        let report = update_dependencies(&config, &installer).unwrap();

        assert_eq!(report.updated.len(), 1);
        assert_eq!(report.updated[0].name, "review");
        assert_eq!(installer.installed.borrow()[0], skill_dir.join("review"));
    }

    fn config(skill_dir: PathBuf, install_source: InstallSource) -> ResolvedConfig {
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
                agents: vec![AgentKind::Pi],
            }],
        }
    }
}
