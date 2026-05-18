use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::application::config::{
    ConfigResolveError, GitInstallSource, InstallSource, RegistryInstallSource, ResolvedAgent,
    ResolvedConfig, ResolvedSkill,
};
use crate::application::ports::{
    display_path, ConfigStore, ConfigStoreError, LockfileStore, LockfileStoreError,
};
use crate::domain::agent::AgentKind;
use crate::domain::lockfile::{
    Digest, LinkType, LockedFile, LockedSkill, LockedTarget, Lockfile, SUPPORTED_LOCKFILE_VERSION,
};
use crate::domain::scope::Scope;
use crate::domain::skill::{SkillName, SourcePath};
use crate::domain::target::TargetPath;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawConfig {
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    #[serde(default = "default_skill_dir")]
    pub skill_dir: PathBuf,
    #[serde(default)]
    pub agents: BTreeMap<String, RawAgentConfig>,
    #[serde(default)]
    pub skills: BTreeMap<String, RawSkillConfig>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, RawDependencyConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAgentConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_scope")]
    pub scope: String,
    pub target_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSkillConfig {
    pub source: Option<PathBuf>,
    #[serde(default)]
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDependencyConfig {
    pub source: RawInstallSource,
    #[serde(default)]
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RawInstallSource {
    Shorthand(String),
    Structured(RawStructuredInstallSource),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawStructuredInstallSource {
    pub provider: Option<String>,
    pub url: Option<String>,
    pub repo: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAgentMappings {
    #[serde(default)]
    agents: BTreeMap<String, RawAgentTargetMapping>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAgentTargetMapping {
    target_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum AgentMappingJsonError {
    #[error("failed to read agent mapping at {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse agent mapping at {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

pub fn read_agent_mappings(
    path: impl AsRef<Path>,
) -> Result<BTreeMap<String, PathBuf>, AgentMappingJsonError> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path).map_err(|source| AgentMappingJsonError::Read {
        path: display_path(path),
        source,
    })?;
    let raw = serde_json::from_str::<RawAgentMappings>(&content).map_err(|source| {
        AgentMappingJsonError::Parse {
            path: display_path(path),
            source,
        }
    })?;
    Ok(raw
        .agents
        .into_iter()
        .map(|(name, mapping)| (name, mapping.target_dir))
        .collect())
}

fn default_enabled() -> bool {
    true
}

fn default_scope() -> String {
    "user".to_owned()
}

fn default_skill_dir() -> PathBuf {
    PathBuf::from("./skills")
}

impl RawConfig {
    pub fn resolve(self) -> Result<ResolvedConfig, ConfigResolveError> {
        let skill_dir = SourcePath::new(self.skill_dir)?;
        let mut agents = BTreeMap::new();
        let mut known_agents = BTreeSet::new();

        for (name, raw_agent) in self.agents {
            upsert_agent(&mut agents, &mut known_agents, name, raw_agent)?;
        }

        let mut skills = Vec::with_capacity(self.skills.len() + self.dependencies.len());
        for (name, raw_skill) in self.skills {
            let skill_name = parse_skill_name(&name)?;
            let source_path = raw_skill
                .source
                .unwrap_or_else(|| skill_dir.as_path().join(skill_name.as_str()));
            let source = SourcePath::new(source_path).map_err(|source| {
                ConfigResolveError::InvalidSkillSource {
                    skill: name.clone(),
                    source,
                }
            })?;
            let skill_agents = resolve_skill_agents(&name, raw_skill.agents, &known_agents)?;

            skills.push(ResolvedSkill {
                name: skill_name,
                source,
                install_source: None,
                agents: skill_agents,
            });
        }

        for (name, raw_dependency) in self.dependencies {
            let skill_name = parse_skill_name(&name)?;
            let source = SourcePath::new(skill_dir.as_path().join(skill_name.as_str())).map_err(
                |source| ConfigResolveError::InvalidSkillSource {
                    skill: name.clone(),
                    source,
                },
            )?;
            let skill_agents = resolve_or_create_dependency_agents(
                &name,
                raw_dependency.agents,
                &mut agents,
                &mut known_agents,
            )?;
            if skill_agents.is_empty() {
                return Err(ConfigResolveError::MissingAgents { skill: name });
            }
            let install_source = parse_install_source(&name, raw_dependency.source)?;

            skills.push(ResolvedSkill {
                name: skill_name,
                source,
                install_source: Some(install_source),
                agents: skill_agents,
            });
        }

        Ok(ResolvedConfig {
            skill_dir,
            agents,
            skills,
        })
    }
}

fn upsert_agent(
    agents: &mut BTreeMap<String, ResolvedAgent>,
    known_agents: &mut BTreeSet<String>,
    name: String,
    raw_agent: RawAgentConfig,
) -> Result<(), ConfigResolveError> {
    let kind = parse_agent_kind(&name)?;
    let key = kind.as_str().to_owned();
    let scope =
        Scope::from_str(&raw_agent.scope).map_err(|source| ConfigResolveError::InvalidScope {
            agent: name.clone(),
            source,
        })?;

    known_agents.insert(key.clone());
    agents.insert(
        key,
        ResolvedAgent {
            kind,
            enabled: raw_agent.enabled,
            scope,
            target_dir: raw_agent.target_dir,
        },
    );
    Ok(())
}

fn parse_skill_name(name: &str) -> Result<SkillName, ConfigResolveError> {
    SkillName::new(name.to_owned()).map_err(|source| ConfigResolveError::InvalidSkillName {
        name: name.to_owned(),
        source,
    })
}

fn resolve_skill_agents(
    skill: &str,
    raw_agents: Vec<String>,
    known_agents: &BTreeSet<String>,
) -> Result<Vec<AgentKind>, ConfigResolveError> {
    let mut skill_agents = Vec::with_capacity(raw_agents.len());
    for agent in raw_agents {
        let kind = parse_agent_kind(&agent)?;
        if !known_agents.contains(kind.as_str()) {
            return Err(ConfigResolveError::UnknownAgent {
                skill: skill.to_owned(),
                agent,
            });
        }
        skill_agents.push(kind);
    }
    Ok(skill_agents)
}

fn resolve_or_create_dependency_agents(
    skill: &str,
    raw_agents: Vec<String>,
    agents: &mut BTreeMap<String, ResolvedAgent>,
    known_agents: &mut BTreeSet<String>,
) -> Result<Vec<AgentKind>, ConfigResolveError> {
    let mut skill_agents = Vec::with_capacity(raw_agents.len());
    for agent in raw_agents {
        let kind = parse_agent_kind(&agent)?;
        let key = kind.as_str().to_owned();
        if !known_agents.contains(&key) {
            known_agents.insert(key.clone());
            agents.insert(
                key,
                ResolvedAgent {
                    kind: kind.clone(),
                    enabled: true,
                    scope: Scope::User,
                    target_dir: None,
                },
            );
        }
        skill_agents.push(kind);
    }
    if skill_agents.is_empty() {
        return Err(ConfigResolveError::MissingAgents {
            skill: skill.to_owned(),
        });
    }
    Ok(skill_agents)
}

fn parse_agent_kind(name: &str) -> Result<AgentKind, ConfigResolveError> {
    AgentKind::from_str(name).map_err(|source| ConfigResolveError::InvalidAgentName {
        name: name.to_owned(),
        source,
    })
}

fn parse_install_source(
    skill: &str,
    raw: RawInstallSource,
) -> Result<InstallSource, ConfigResolveError> {
    match raw {
        RawInstallSource::Shorthand(value) => parse_install_source_string(skill, &value),
        RawInstallSource::Structured(source) => parse_structured_install_source(skill, source),
    }
}

fn parse_structured_install_source(
    skill: &str,
    source: RawStructuredInstallSource,
) -> Result<InstallSource, ConfigResolveError> {
    match source.provider.as_deref() {
        Some("local") => {
            let path = source
                .path
                .ok_or_else(|| ConfigResolveError::InvalidInstallSource {
                    skill: skill.to_owned(),
                    message: "local source requires path".to_owned(),
                })?;
            Ok(InstallSource::Local(path))
        }
        Some("registry") => {
            let registry = source.url.unwrap_or_else(|| "skills.sh".to_owned());
            let package = source
                .repo
                .ok_or_else(|| ConfigResolveError::InvalidInstallSource {
                    skill: skill.to_owned(),
                    message: "registry source requires repo/package".to_owned(),
                })?;
            Ok(InstallSource::Registry(RegistryInstallSource {
                registry,
                package,
                reference: source.reference,
            }))
        }
        _ => {
            let repo = source.repo.or(source.url).ok_or_else(|| {
                ConfigResolveError::InvalidInstallSource {
                    skill: skill.to_owned(),
                    message: "git source requires repo or url".to_owned(),
                }
            })?;
            let path = source.path.unwrap_or_else(|| PathBuf::from("."));
            Ok(InstallSource::Git(GitInstallSource {
                url: git_url_from_repo(&repo),
                reference: source.reference,
                path,
            }))
        }
    }
}

fn parse_install_source_string(
    skill: &str,
    value: &str,
) -> Result<InstallSource, ConfigResolveError> {
    if value.starts_with("./") || value.starts_with("../") || value.starts_with('/') {
        return Ok(InstallSource::Local(PathBuf::from(value)));
    }

    if let Some(parsed) = parse_github_tree_url(value) {
        return Ok(InstallSource::Git(parsed));
    }

    let (body, reference) = split_ref(value);
    if let Some(package) = body.strip_prefix("registry:") {
        return Ok(InstallSource::Registry(RegistryInstallSource {
            registry: "skills.sh".to_owned(),
            package: package.to_owned(),
            reference: reference.map(str::to_owned),
        }));
    }
    if body.starts_with("skills.sh/") {
        return Ok(InstallSource::Registry(RegistryInstallSource {
            registry: "skills.sh".to_owned(),
            package: body.trim_start_matches("skills.sh/").to_owned(),
            reference: reference.map(str::to_owned),
        }));
    }
    let body = body.strip_prefix("github:").unwrap_or(body);
    let parts: Vec<&str> = body.split('/').filter(|part| !part.is_empty()).collect();
    if parts.len() >= 2 && !body.contains("://") {
        let repo = format!("{}/{}", parts[0], parts[1]);
        let path = if parts.len() > 2 {
            parts[2..].join("/")
        } else {
            ".".to_owned()
        };
        return Ok(InstallSource::Git(GitInstallSource {
            url: git_url_from_repo(&repo),
            reference: reference.map(str::to_owned),
            path: PathBuf::from(path),
        }));
    }

    Err(ConfigResolveError::InvalidInstallSource {
        skill: skill.to_owned(),
        message: format!("unsupported source '{value}'"),
    })
}

fn split_ref(value: &str) -> (&str, Option<&str>) {
    value
        .rsplit_once('#')
        .map_or((value, None), |(body, reference)| (body, Some(reference)))
}

fn parse_github_tree_url(value: &str) -> Option<GitInstallSource> {
    let prefix = "https://github.com/";
    let rest = value.strip_prefix(prefix)?;
    let (body, reference_override) = split_ref(rest);
    let parts: Vec<&str> = body.split('/').filter(|part| !part.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    let repo = format!("{}/{}", parts[0], parts[1]);
    let mut reference = reference_override.map(str::to_owned);
    let mut path = PathBuf::from(".");
    if parts.get(2) == Some(&"tree") && parts.len() >= 4 {
        reference = Some(parts[3].to_owned());
        if parts.len() > 4 {
            path = PathBuf::from(parts[4..].join("/"));
        }
    }
    Some(GitInstallSource {
        url: git_url_from_repo(&repo),
        reference,
        path,
    })
}

fn git_url_from_repo(repo_or_url: &str) -> String {
    if repo_or_url.contains("://") || repo_or_url.ends_with(".git") {
        repo_or_url.to_owned()
    } else {
        format!("https://github.com/{repo_or_url}.git")
    }
}

#[derive(Debug, Clone)]
pub struct FileConfigStore {
    path: PathBuf,
}

impl FileConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConfigStore for FileConfigStore {
    fn load(&self) -> Result<ResolvedConfig, ConfigStoreError> {
        let content =
            std::fs::read_to_string(&self.path).map_err(|source| ConfigStoreError::Read {
                path: display_path(&self.path),
                source,
            })?;
        let raw = serde_json::from_str::<RawConfig>(&content).map_err(|source| {
            ConfigStoreError::Parse {
                path: display_path(&self.path),
                source,
            }
        })?;

        raw.resolve().map_err(ConfigStoreError::from)
    }
}

#[derive(Debug, Error)]
pub enum LockfileJsonError {
    #[error("failed to read lockfile at {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write lockfile at {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse lockfile at {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize lockfile: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("unsupported lockfileVersion {found}; supported version is {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("invalid lockfile field: {0}")]
    InvalidField(String),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawLockfile {
    lockfile_version: u32,
    generated_by: String,
    generated_at: String,
    root: PathBuf,
    skills: BTreeMap<String, RawLockedSkill>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawLockedSkill {
    source: PathBuf,
    hash: String,
    files: Vec<RawLockedFile>,
    targets: Vec<RawLockedTarget>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawLockedFile {
    path: PathBuf,
    hash: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawLockedTarget {
    agent: String,
    scope: String,
    path: PathBuf,
    link_type: String,
}

pub fn read_lockfile(path: impl AsRef<Path>) -> Result<Lockfile, LockfileJsonError> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path).map_err(|source| LockfileJsonError::Read {
        path: display_path(path),
        source,
    })?;
    let value = serde_json::from_str::<serde_json::Value>(&content).map_err(|source| {
        LockfileJsonError::Parse {
            path: display_path(path),
            source,
        }
    })?;
    reject_unsupported_lockfile_version(&value)?;
    let raw = serde_json::from_value::<RawLockfile>(value).map_err(|source| {
        LockfileJsonError::Parse {
            path: display_path(path),
            source,
        }
    })?;

    raw.try_into_domain()
}

fn reject_unsupported_lockfile_version(value: &serde_json::Value) -> Result<(), LockfileJsonError> {
    let Some(found) = value
        .get("lockfileVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
    else {
        return Ok(());
    };

    if found != SUPPORTED_LOCKFILE_VERSION {
        return Err(LockfileJsonError::UnsupportedVersion {
            found,
            supported: SUPPORTED_LOCKFILE_VERSION,
        });
    }

    Ok(())
}

pub fn write_lockfile(
    path: impl AsRef<Path>,
    lockfile: &Lockfile,
) -> Result<(), LockfileJsonError> {
    let path = path.as_ref();
    let raw = RawLockfile::from_domain(lockfile);
    let content = serde_json::to_string_pretty(&raw)?;
    std::fs::write(path, format!("{content}\n")).map_err(|source| LockfileJsonError::Write {
        path: display_path(path),
        source,
    })
}

#[derive(Debug, Clone)]
pub struct FileLockfileStore {
    path: PathBuf,
}

impl FileLockfileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl LockfileStore for FileLockfileStore {
    fn write(&self, lockfile: &Lockfile) -> Result<(), LockfileStoreError> {
        write_lockfile(&self.path, lockfile)
            .map_err(|error| LockfileStoreError::Write(error.to_string()))
    }
}

impl RawLockfile {
    fn try_into_domain(self) -> Result<Lockfile, LockfileJsonError> {
        if self.lockfile_version != SUPPORTED_LOCKFILE_VERSION {
            return Err(LockfileJsonError::UnsupportedVersion {
                found: self.lockfile_version,
                supported: SUPPORTED_LOCKFILE_VERSION,
            });
        }

        let mut skills = BTreeMap::new();
        for (name, raw_skill) in self.skills {
            let skill_name = SkillName::new(name.clone()).map_err(|source| {
                LockfileJsonError::InvalidField(format!("skill name '{name}': {source}"))
            })?;
            let source = SourcePath::new(raw_skill.source).map_err(|source| {
                LockfileJsonError::InvalidField(format!("skill '{name}' source: {source}"))
            })?;
            let hash = Digest::new(raw_skill.hash).map_err(|source| {
                LockfileJsonError::InvalidField(format!("skill '{name}' hash: {source}"))
            })?;
            let files = raw_skill
                .files
                .into_iter()
                .map(|file| {
                    Ok(LockedFile {
                        path: file.path,
                        hash: Digest::new(file.hash).map_err(|source| {
                            LockfileJsonError::InvalidField(format!(
                                "skill '{name}' file hash: {source}"
                            ))
                        })?,
                    })
                })
                .collect::<Result<Vec<_>, LockfileJsonError>>()?;
            let targets = raw_skill
                .targets
                .into_iter()
                .map(|target| {
                    Ok(LockedTarget {
                        agent: AgentKind::from_str(&target.agent).map_err(|source| {
                            LockfileJsonError::InvalidField(format!(
                                "target agent '{}': {source}",
                                target.agent
                            ))
                        })?,
                        scope: Scope::from_str(&target.scope).map_err(|source| {
                            LockfileJsonError::InvalidField(format!(
                                "target scope '{}': {source}",
                                target.scope
                            ))
                        })?,
                        path: TargetPath::new(target.path).map_err(|source| {
                            LockfileJsonError::InvalidField(format!("target path: {source}"))
                        })?,
                        link_type: LinkType::from_str(&target.link_type).map_err(|source| {
                            LockfileJsonError::InvalidField(format!(
                                "target linkType '{}': {source}",
                                target.link_type
                            ))
                        })?,
                    })
                })
                .collect::<Result<Vec<_>, LockfileJsonError>>()?;

            skills.insert(
                skill_name,
                LockedSkill {
                    source,
                    hash,
                    files,
                    targets,
                },
            );
        }

        Ok(Lockfile {
            generated_by: self.generated_by,
            generated_at: self.generated_at,
            root: self.root,
            skills,
        })
    }

    fn from_domain(lockfile: &Lockfile) -> Self {
        let skills = lockfile
            .skills
            .iter()
            .map(|(name, skill)| {
                (
                    name.as_str().to_owned(),
                    RawLockedSkill {
                        source: skill.source.as_path().to_path_buf(),
                        hash: skill.hash.as_str().to_owned(),
                        files: skill
                            .files
                            .iter()
                            .map(|file| RawLockedFile {
                                path: file.path.clone(),
                                hash: file.hash.as_str().to_owned(),
                            })
                            .collect(),
                        targets: skill
                            .targets
                            .iter()
                            .map(|target| RawLockedTarget {
                                agent: target.agent.as_str().to_owned(),
                                scope: target.scope.as_str().to_owned(),
                                path: target.path.as_path().to_path_buf(),
                                link_type: target.link_type.as_str().to_owned(),
                            })
                            .collect(),
                    },
                )
            })
            .collect();

        Self {
            lockfile_version: SUPPORTED_LOCKFILE_VERSION,
            generated_by: lockfile.generated_by.clone(),
            generated_at: lockfile.generated_at.clone(),
            root: lockfile.root.clone(),
            skills,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{read_lockfile, write_lockfile, FileConfigStore, LockfileJsonError, RawConfig};
    use crate::application::config::ConfigResolveError;
    use crate::application::ports::ConfigStore;
    use crate::domain::agent::AgentKind;
    use crate::domain::lockfile::SUPPORTED_LOCKFILE_VERSION;
    use crate::domain::scope::Scope;
    use std::path::Path;

    #[test]
    fn parses_example_config() {
        let raw =
            serde_json::from_str::<RawConfig>(include_str!("../../sksync.config.example.json"))
                .expect("example config parses");
        let config = raw.resolve().expect("example config resolves");

        assert_eq!(config.skill_dir.as_path(), Path::new("./skills"));
        assert_eq!(config.agents.len(), 5);
        assert_eq!(config.skills.len(), 2);
        assert_eq!(config.skills[0].name.as_str(), "example-skill");
        assert_eq!(config.skills[0].agents.len(), 5);
        assert!(config.skills[0].install_source.is_some());
        assert_eq!(config.agents["pi"].kind, AgentKind::Pi);
        assert_eq!(config.agents["pi"].scope, Scope::User);
    }

    #[test]
    fn rejects_missing_agent_reference() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./skills",
              "agents": { "pi": { "enabled": true, "scope": "user" } },
              "skills": { "review": { "agents": ["missing"] } }
            }"#,
        )
        .expect("raw config parses");

        assert_eq!(
            raw.resolve(),
            Err(ConfigResolveError::UnknownAgent {
                skill: "review".to_owned(),
                agent: "missing".to_owned(),
            })
        );
    }

    #[test]
    fn fills_missing_skill_source_from_skill_dir_and_name() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./skills",
              "agents": { "pi": { "enabled": true, "scope": "project" } },
              "skills": { "review": { "agents": ["pi"] } }
            }"#,
        )
        .expect("raw config parses");
        let config = raw.resolve().expect("config resolves");

        assert_eq!(
            config.skills[0].source.as_path(),
            Path::new("./skills/review")
        );
    }

    #[test]
    fn file_config_store_loads_config_from_disk() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            include_str!("../../sksync.config.example.json"),
        )
        .expect("write config fixture");

        let store = FileConfigStore::new(&config_path);
        let config = store.load().expect("file config loads");

        assert_eq!(store.path(), config_path.as_path());
        assert_eq!(config.skills[0].name.as_str(), "example-skill");
    }

    #[test]
    fn parses_example_lockfile() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let lockfile_path = temp_dir.path().join("sksync-lock.json");
        std::fs::write(
            &lockfile_path,
            include_str!("../../sksync-lock.example.json"),
        )
        .expect("write lockfile fixture");

        let lockfile = read_lockfile(&lockfile_path).expect("example lockfile parses");
        let (name, skill) = lockfile.skills.iter().next().expect("one locked skill");

        assert_eq!(lockfile.generated_by, "sksync@0.1.0");
        assert_eq!(lockfile.root, Path::new("."));
        assert_eq!(name.as_str(), "example-skill");
        assert_eq!(skill.hash.as_str(), "sha256-placeholder");
        assert_eq!(skill.targets[0].agent, AgentKind::Pi);
        assert_eq!(skill.targets[0].scope, Scope::User);
    }

    #[test]
    fn lockfile_roundtrips_through_json_file() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let source_path = temp_dir.path().join("source-lock.json");
        let roundtrip_path = temp_dir.path().join("roundtrip-lock.json");
        std::fs::write(&source_path, include_str!("../../sksync-lock.example.json"))
            .expect("write source lockfile");
        let lockfile = read_lockfile(&source_path).expect("read source lockfile");

        write_lockfile(&roundtrip_path, &lockfile).expect("write lockfile");
        let reread = read_lockfile(&roundtrip_path).expect("read roundtrip lockfile");

        assert_eq!(reread, lockfile);
    }

    #[test]
    fn rejects_unsupported_lockfile_version() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let lockfile_path = temp_dir.path().join("sksync-lock.json");
        std::fs::write(
            &lockfile_path,
            include_str!("../../sksync-lock.example.json")
                .replace("\"lockfileVersion\": 1", "\"lockfileVersion\": 2"),
        )
        .expect("write lockfile fixture");

        let error = read_lockfile(&lockfile_path).expect_err("unsupported version fails");
        assert!(matches!(
            error,
            LockfileJsonError::UnsupportedVersion {
                found: 2,
                supported: SUPPORTED_LOCKFILE_VERSION,
            }
        ));
    }
}
