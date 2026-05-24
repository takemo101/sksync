use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::application::config::{
    ConfigResolveError, ResolvedAgent, ResolvedConfig, ResolvedSkill,
};
use crate::application::ports::{
    display_path, ConfigStore, ConfigStoreError, DependencyConfigStore, DependencyConfigStoreError,
    LockfileStore, LockfileStoreError,
};
use crate::application::source::{
    git_url_from_repo, parse_install_source_string as parse_source_string, validate_git_subpath,
};
use crate::domain::agent::AgentKind;
use crate::domain::lockfile::{
    Digest, LinkType, LockedFile, LockedSkill, LockedTarget, Lockfile, LEGACY_LOCKFILE_VERSION,
    LEGACY_LOCKFILE_VERSION_WITH_TARGETS, SUPPORTED_LOCKFILE_VERSION,
};
use crate::domain::scope::Scope;
use crate::domain::skill::{SkillName, SourcePath, SourcePathError};
use crate::domain::source::{GitInstallSource, InstallSource};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMappingConfig {
    pub global: BTreeMap<String, PathBuf>,
    pub project: BTreeMap<String, PathBuf>,
}

impl AgentMappingConfig {
    pub fn merge(&mut self, other: Self) {
        self.global.extend(other.global);
        self.project.extend(other.project);
    }
}

#[derive(Debug, Deserialize)]
struct RawAgentMappings {
    #[serde(default)]
    global: BTreeMap<String, RawAgentTargetMapping>,
    #[serde(default)]
    project: BTreeMap<String, RawAgentTargetMapping>,
    #[serde(default, rename = "agents")]
    legacy_global: BTreeMap<String, RawAgentTargetMapping>,
    #[serde(default, rename = "projectAgents")]
    legacy_project: BTreeMap<String, RawAgentTargetMapping>,
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

pub fn default_agent_mapping_config() -> Result<AgentMappingConfig, AgentMappingJsonError> {
    parse_agent_mapping_config(
        include_str!("../../sksync.agents.example.json"),
        "sksync.agents.example.json",
    )
}

pub fn read_agent_mapping_config(
    path: impl AsRef<Path>,
) -> Result<AgentMappingConfig, AgentMappingJsonError> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path).map_err(|source| AgentMappingJsonError::Read {
        path: display_path(path),
        source,
    })?;
    parse_agent_mapping_config(&content, &display_path(path))
}

pub fn read_agent_mappings(
    path: impl AsRef<Path>,
) -> Result<BTreeMap<String, PathBuf>, AgentMappingJsonError> {
    read_agent_mapping_config(path).map(|config| config.global)
}

fn parse_agent_mapping_config(
    content: &str,
    path: &str,
) -> Result<AgentMappingConfig, AgentMappingJsonError> {
    let raw = serde_json::from_str::<RawAgentMappings>(content).map_err(|source| {
        AgentMappingJsonError::Parse {
            path: path.to_owned(),
            source,
        }
    })?;

    let mut global = raw
        .legacy_global
        .into_iter()
        .map(|(name, mapping)| (name, mapping.target_dir))
        .collect::<BTreeMap<_, _>>();
    global.extend(
        raw.global
            .into_iter()
            .map(|(name, mapping)| (name, mapping.target_dir)),
    );

    let mut project = raw
        .legacy_project
        .into_iter()
        .map(|(name, mapping)| (name, mapping.target_dir))
        .collect::<BTreeMap<_, _>>();
    project.extend(
        raw.project
            .into_iter()
            .map(|(name, mapping)| (name, mapping.target_dir)),
    );

    Ok(AgentMappingConfig { global, project })
}

fn default_enabled() -> bool {
    true
}

fn default_scope() -> String {
    "user".to_owned()
}

fn default_skill_dir() -> PathBuf {
    PathBuf::from("./.sksync/skills")
}

impl RawConfig {
    pub fn resolve(self) -> Result<ResolvedConfig, ConfigResolveError> {
        self.resolve_with_default_scope(Scope::User)
    }

    pub fn resolve_with_default_scope(
        self,
        default_dependency_scope: Scope,
    ) -> Result<ResolvedConfig, ConfigResolveError> {
        self.resolve_with_default_scope_and_root(default_dependency_scope, None)
    }

    fn resolve_with_default_scope_and_root(
        self,
        default_dependency_scope: Scope,
        config_root: Option<&Path>,
    ) -> Result<ResolvedConfig, ConfigResolveError> {
        let skill_dir = SourcePath::new(rebase_config_path(self.skill_dir, config_root))?;
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
                .map(|source| rebase_config_path(source, config_root))
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
            let skill_agents = resolve_or_create_dependency_agents(
                &name,
                raw_dependency.agents,
                &mut agents,
                &mut known_agents,
                default_dependency_scope,
            )?;
            if skill_agents.is_empty() {
                return Err(ConfigResolveError::MissingAgents { skill: name });
            }
            let install_source = parse_install_source(&name, raw_dependency.source, config_root)?;
            let source = dependency_source_path(&skill_dir, skill_name.as_str(), &install_source)
                .map_err(|source| ConfigResolveError::InvalidSkillSource {
                skill: name.clone(),
                source,
            })?;

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

fn rebase_config_path(path: PathBuf, config_root: Option<&Path>) -> PathBuf {
    if path.is_absolute() || is_tilde_path(&path) {
        return path;
    }
    config_root.map(|root| root.join(&path)).unwrap_or(path)
}

fn is_tilde_path(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|value| value == "~" || value.starts_with("~/"))
}

fn rebase_lockfile_path(path: PathBuf, lockfile_root: &Path) -> PathBuf {
    if path.is_absolute() || is_tilde_path(&path) {
        path
    } else {
        lockfile_root.join(path)
    }
}

fn relativize_lockfile_path(path: &Path, lockfile_root: &Path) -> PathBuf {
    path.strip_prefix(lockfile_root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
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
    let mut seen_agents = BTreeSet::new();
    for agent in raw_agents {
        let kind = parse_agent_kind(&agent)?;
        let key = kind.as_str().to_owned();
        if !known_agents.contains(&key) {
            return Err(ConfigResolveError::UnknownAgent {
                skill: skill.to_owned(),
                agent,
            });
        }
        if seen_agents.insert(key) {
            skill_agents.push(kind);
        }
    }
    Ok(skill_agents)
}

fn resolve_or_create_dependency_agents(
    skill: &str,
    raw_agents: Vec<String>,
    agents: &mut BTreeMap<String, ResolvedAgent>,
    known_agents: &mut BTreeSet<String>,
    default_scope: Scope,
) -> Result<Vec<AgentKind>, ConfigResolveError> {
    let mut skill_agents = Vec::with_capacity(raw_agents.len());
    let mut seen_agents = BTreeSet::new();
    for agent in raw_agents {
        let kind = parse_agent_kind(&agent)?;
        let key = kind.as_str().to_owned();
        if !known_agents.contains(&key) {
            known_agents.insert(key.clone());
            agents.insert(
                key.clone(),
                ResolvedAgent {
                    kind: kind.clone(),
                    enabled: true,
                    scope: default_scope,
                    target_dir: None,
                },
            );
        }
        if seen_agents.insert(key) {
            skill_agents.push(kind);
        }
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
    config_root: Option<&Path>,
) -> Result<InstallSource, ConfigResolveError> {
    match raw {
        RawInstallSource::Shorthand(value) => {
            parse_install_source_value(skill, &value, config_root)
        }
        RawInstallSource::Structured(source) => {
            parse_structured_install_source(skill, source, config_root)
        }
    }
}

fn parse_structured_install_source(
    skill: &str,
    source: RawStructuredInstallSource,
    config_root: Option<&Path>,
) -> Result<InstallSource, ConfigResolveError> {
    match source.provider.as_deref() {
        Some("local") => {
            let path = source
                .path
                .ok_or_else(|| ConfigResolveError::InvalidInstallSource {
                    skill: skill.to_owned(),
                    message: "local source requires path".to_owned(),
                })?;
            Ok(InstallSource::Local(rebase_config_path(path, config_root)))
        }
        Some("registry") => Err(ConfigResolveError::InvalidInstallSource {
            skill: skill.to_owned(),
            message: "registry sources are not supported; use a provider URL such as https://www.skills.sh/owner/repo/skill-name".to_owned(),
        }),
        _ => {
            let repo = source.repo.or(source.url).ok_or_else(|| {
                ConfigResolveError::InvalidInstallSource {
                    skill: skill.to_owned(),
                    message: "git source requires repo or url".to_owned(),
                }
            })?;
            let path = source.path.unwrap_or_else(|| PathBuf::from("."));
            let path = validate_git_subpath(path).map_err(|error| {
                ConfigResolveError::InvalidInstallSource {
                    skill: skill.to_owned(),
                    message: error.to_string(),
                }
            })?;
            Ok(InstallSource::Git(GitInstallSource {
                url: git_url_from_repo(&repo),
                reference: source.reference,
                path,
            }))
        }
    }
}

fn dependency_source_path(
    skill_dir: &SourcePath,
    skill_name: &str,
    install_source: &InstallSource,
) -> Result<SourcePath, SourcePathError> {
    let legacy_flat_path = skill_dir.as_path().join(skill_name);
    if legacy_flat_path.exists() {
        return SourcePath::new(legacy_flat_path);
    }

    SourcePath::new(
        skill_dir
            .as_path()
            .join(install_source.storage_subpath(skill_name)),
    )
}

pub fn parse_install_source_value(
    skill: &str,
    value: &str,
    config_root: Option<&Path>,
) -> Result<InstallSource, ConfigResolveError> {
    let source =
        parse_source_string(value).map_err(|error| ConfigResolveError::InvalidInstallSource {
            skill: skill.to_owned(),
            message: error.to_string(),
        })?;
    Ok(match source {
        InstallSource::Local(path) => InstallSource::Local(rebase_config_path(path, config_root)),
        InstallSource::Git(source) => InstallSource::Git(source),
    })
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

    pub fn load_with_default_scope(
        &self,
        default_dependency_scope: Scope,
    ) -> Result<ResolvedConfig, ConfigStoreError> {
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

        let config_root = config_root_for_path(&self.path);
        raw.resolve_with_default_scope_and_root(default_dependency_scope, Some(config_root))
            .map_err(ConfigStoreError::from)
    }
}

#[derive(Debug, Clone)]
pub struct FileDependencyConfigStore {
    path: PathBuf,
    default_skill_dir: PathBuf,
}

impl FileDependencyConfigStore {
    pub fn new(path: impl Into<PathBuf>, default_skill_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            default_skill_dir: default_skill_dir.into(),
        }
    }
}

impl DependencyConfigStore for FileDependencyConfigStore {
    fn add_dependency(
        &self,
        skill_name: &str,
        source: &str,
        agents: &[String],
    ) -> Result<(), DependencyConfigStoreError> {
        let mut value = self.load_or_default()?;
        let dependencies = dependencies_object_mut(&mut value)?;
        let mut merged_agents = dependency_agents(dependencies.get(skill_name), skill_name)?;
        let mut normalized_existing_agents = merged_agents
            .iter()
            .map(|agent| normalize_agent_name(agent))
            .collect::<BTreeSet<_>>();
        for agent in agents {
            let normalized = normalize_agent_name(agent);
            if normalized_existing_agents.insert(normalized.clone()) {
                merged_agents.push(normalized);
            }
        }
        dependencies.insert(
            skill_name.to_owned(),
            json!({
                "source": source,
                "agents": merged_agents,
            }),
        );
        self.write_value(&value)
    }

    fn add_dependency_agents(
        &self,
        skill_name: &str,
        agents: &[String],
    ) -> Result<Vec<String>, DependencyConfigStoreError> {
        let mut value = self.load_or_default()?;
        let dependencies = dependencies_object_mut(&mut value)?;
        let dependency = dependencies.get_mut(skill_name).ok_or_else(|| {
            DependencyConfigStoreError::InvalidField(format!(
                "dependencies.{skill_name} must exist before attaching agents"
            ))
        })?;
        let mut merged_agents = dependency_agents(Some(dependency), skill_name)?;
        let mut normalized_existing_agents = merged_agents
            .iter()
            .map(|agent| normalize_agent_name(agent))
            .collect::<BTreeSet<_>>();
        for agent in agents {
            let normalized = normalize_agent_name(agent);
            if normalized_existing_agents.insert(normalized.clone()) {
                merged_agents.push(normalized);
            }
        }
        dependency
            .as_object_mut()
            .ok_or_else(|| {
                DependencyConfigStoreError::InvalidField(format!(
                    "dependencies.{skill_name} must be an object"
                ))
            })?
            .insert("agents".to_owned(), json!(merged_agents.clone()));
        self.write_value(&value)?;
        Ok(merged_agents)
    }

    fn remove_dependency(&self, skill_name: &str) -> Result<(), DependencyConfigStoreError> {
        if !self.path.exists() {
            return Ok(());
        }
        let mut value = self.load_or_default()?;
        dependencies_object_mut(&mut value)?.remove(skill_name);
        self.write_value(&value)
    }

    fn remove_dependency_agents(
        &self,
        skill_name: &str,
        agents: &[String],
    ) -> Result<Vec<String>, DependencyConfigStoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let mut value = self.load_or_default()?;
        let dependencies = dependencies_object_mut(&mut value)?;
        let mut remaining_agents = dependency_agents(dependencies.get(skill_name), skill_name)?;
        let normalized_removed_agents = agents
            .iter()
            .map(|agent| normalize_agent_name(agent))
            .collect::<Vec<_>>();
        remaining_agents.retain(|agent| {
            let normalized_agent = normalize_agent_name(agent);
            !normalized_removed_agents
                .iter()
                .any(|removed| removed == &normalized_agent)
        });
        if remaining_agents.is_empty() {
            dependencies.remove(skill_name);
        } else if let Some(dependency) = dependencies.get_mut(skill_name) {
            dependency
                .as_object_mut()
                .ok_or_else(|| {
                    DependencyConfigStoreError::InvalidField(format!(
                        "dependencies.{skill_name} must be an object"
                    ))
                })?
                .insert("agents".to_owned(), json!(remaining_agents.clone()));
        }
        self.write_value(&value)?;
        Ok(remaining_agents)
    }
}

impl FileDependencyConfigStore {
    fn load_or_default(&self) -> Result<serde_json::Value, DependencyConfigStoreError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| {
                DependencyConfigStoreError::CreateDir {
                    path: display_path(parent),
                    source,
                }
            })?;
        }
        if self.path.exists() {
            let content = std::fs::read_to_string(&self.path).map_err(|source| {
                DependencyConfigStoreError::Read {
                    path: display_path(&self.path),
                    source,
                }
            })?;
            serde_json::from_str::<serde_json::Value>(&content).map_err(|source| {
                DependencyConfigStoreError::Parse {
                    path: display_path(&self.path),
                    source,
                }
            })
        } else {
            Ok(json!({
                "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.schema.json",
                "skillDir": self.default_skill_dir,
                "dependencies": {}
            }))
        }
    }

    fn write_value(&self, value: &serde_json::Value) -> Result<(), DependencyConfigStoreError> {
        let content = serde_json::to_string_pretty(value)?;
        std::fs::write(&self.path, format!("{content}\n")).map_err(|source| {
            DependencyConfigStoreError::Write {
                path: display_path(&self.path),
                source,
            }
        })
    }
}

fn dependencies_object_mut(
    value: &mut serde_json::Value,
) -> Result<&mut serde_json::Map<String, serde_json::Value>, DependencyConfigStoreError> {
    let dependencies = value
        .as_object_mut()
        .ok_or_else(|| {
            DependencyConfigStoreError::InvalidField("config root must be an object".to_owned())
        })?
        .entry("dependencies")
        .or_insert_with(|| json!({}));
    dependencies.as_object_mut().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField("dependencies must be an object".to_owned())
    })
}

fn normalize_agent_name(agent: &str) -> String {
    AgentKind::from_str(agent)
        .map(|agent| agent.as_str().to_owned())
        .unwrap_or_else(|_| agent.trim().to_ascii_lowercase())
}

fn dependency_agents(
    dependency: Option<&serde_json::Value>,
    skill_name: &str,
) -> Result<Vec<String>, DependencyConfigStoreError> {
    let Some(dependency) = dependency else {
        return Ok(Vec::new());
    };
    let dependency = dependency.as_object().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name} must be an object"
        ))
    })?;
    let Some(agents) = dependency.get("agents") else {
        return Ok(Vec::new());
    };
    let agents = agents.as_array().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.agents must be an array"
        ))
    })?;
    agents
        .iter()
        .map(|agent| {
            agent.as_str().map(str::to_owned).ok_or_else(|| {
                DependencyConfigStoreError::InvalidField(format!(
                    "dependencies.{skill_name}.agents must contain only strings"
                ))
            })
        })
        .collect()
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

        let config_root = config_root_for_path(&self.path);
        raw.resolve_with_default_scope_and_root(Scope::User, Some(config_root))
            .map_err(ConfigStoreError::from)
    }
}

fn config_root_for_path(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
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
    #[serde(default)]
    install_source: Option<RawLockedInstallSource>,
    hash: String,
    files: Vec<RawLockedFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    targets: Vec<RawLockedTarget>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum RawLockedInstallSource {
    Git {
        url: String,
        #[serde(rename = "ref")]
        reference: Option<String>,
        path: PathBuf,
    },
    Local {
        path: PathBuf,
    },
}

impl RawLockedInstallSource {
    fn try_into_domain(self, lockfile_root: &Path) -> Result<InstallSource, LockfileJsonError> {
        Ok(match self {
            Self::Git {
                url,
                reference,
                path,
            } => InstallSource::Git(GitInstallSource {
                url,
                reference,
                path,
            }),
            Self::Local { path } => InstallSource::Local(rebase_lockfile_path(path, lockfile_root)),
        })
    }

    fn from_domain(source: &InstallSource, lockfile_root: &Path) -> Self {
        match source {
            InstallSource::Git(git) => Self::Git {
                url: git.url.clone(),
                reference: git.reference.clone(),
                path: git.path.clone(),
            },
            InstallSource::Local(path) => Self::Local {
                path: relativize_lockfile_path(path, lockfile_root),
            },
        }
    }
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

    let lockfile_root = path.parent().unwrap_or_else(|| Path::new("."));
    raw.try_into_domain(lockfile_root)
}

fn is_supported_lockfile_version(version: u32) -> bool {
    matches!(
        version,
        SUPPORTED_LOCKFILE_VERSION | LEGACY_LOCKFILE_VERSION | LEGACY_LOCKFILE_VERSION_WITH_TARGETS
    )
}

fn reject_unsupported_lockfile_version(value: &serde_json::Value) -> Result<(), LockfileJsonError> {
    let Some(found) = value
        .get("lockfileVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
    else {
        return Ok(());
    };

    if !is_supported_lockfile_version(found) {
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
    let lockfile_root = path.parent().unwrap_or_else(|| Path::new("."));
    let raw = RawLockfile::from_domain(lockfile, lockfile_root);
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
    fn try_into_domain(self, lockfile_root: &Path) -> Result<Lockfile, LockfileJsonError> {
        if !is_supported_lockfile_version(self.lockfile_version) {
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
            let source_path = rebase_lockfile_path(raw_skill.source, lockfile_root);
            let source = SourcePath::new(source_path).map_err(|source| {
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
                    install_source: raw_skill
                        .install_source
                        .map(|source| source.try_into_domain(lockfile_root))
                        .transpose()?,
                    hash,
                    files,
                    targets,
                },
            );
        }

        Ok(Lockfile {
            generated_by: self.generated_by,
            generated_at: self.generated_at,
            root: if self.lockfile_version == SUPPORTED_LOCKFILE_VERSION {
                PathBuf::from(".")
            } else {
                self.root
            },
            skills,
        })
    }

    fn from_domain(lockfile: &Lockfile, lockfile_root: &Path) -> Self {
        let skills = lockfile
            .skills
            .iter()
            .map(|(name, skill)| {
                (
                    name.as_str().to_owned(),
                    RawLockedSkill {
                        source: relativize_lockfile_path(skill.source.as_path(), lockfile_root),
                        install_source: skill.install_source.as_ref().map(|source| {
                            RawLockedInstallSource::from_domain(source, lockfile_root)
                        }),
                        hash: skill.hash.as_str().to_owned(),
                        files: skill
                            .files
                            .iter()
                            .map(|file| RawLockedFile {
                                path: file.path.clone(),
                                hash: file.hash.as_str().to_owned(),
                            })
                            .collect(),
                        targets: Vec::new(),
                    },
                )
            })
            .collect();

        Self {
            lockfile_version: SUPPORTED_LOCKFILE_VERSION,
            generated_by: lockfile.generated_by.clone(),
            generated_at: lockfile.generated_at.clone(),
            root: PathBuf::from("."),
            skills,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_agent_mapping_config, read_lockfile, write_lockfile, FileConfigStore,
        FileDependencyConfigStore, LockfileJsonError, RawConfig,
    };
    use crate::application::config::ConfigResolveError;
    use crate::application::ports::{ConfigStore, DependencyConfigStore};
    use crate::domain::agent::AgentKind;
    use crate::domain::lockfile::{Digest, LockedSkill, Lockfile, SUPPORTED_LOCKFILE_VERSION};
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use crate::domain::source::{GitInstallSource, InstallSource};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    #[test]
    fn parses_example_config() {
        let raw =
            serde_json::from_str::<RawConfig>(include_str!("../../sksync.config.example.json"))
                .expect("example config parses");
        let config = raw.resolve().expect("example config resolves");

        assert_eq!(config.skill_dir.as_path(), Path::new("./.sksync/skills"));
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
              "skillDir": "./.sksync/skills",
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
              "skillDir": "./.sksync/skills",
              "agents": { "pi": { "enabled": true, "scope": "project" } },
              "skills": { "review": { "agents": ["pi"] } }
            }"#,
        )
        .expect("raw config parses");
        let config = raw.resolve().expect("config resolves");

        assert_eq!(
            config.skills[0].source.as_path(),
            Path::new("./.sksync/skills/review")
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
    fn write_lockfile_serializes_portable_v4_paths_relative_to_lockfile_directory() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let root = temp_dir.path();
        let lockfile_path = root.join("sksync-lock.json");
        let source = root.join(".sksync/skills/owner/repo/review");
        let local_source = root.join("vendor/review");
        let mut skills = BTreeMap::new();
        skills.insert(
            SkillName::new("review").expect("skill name"),
            LockedSkill {
                source: SourcePath::new(source).expect("source path"),
                install_source: Some(InstallSource::Local(local_source)),
                hash: Digest::new("sha256-review").expect("hash"),
                files: Vec::new(),
                targets: Vec::new(),
            },
        );
        let lockfile = Lockfile {
            generated_by: "sksync@test".to_owned(),
            generated_at: "unix:1".to_owned(),
            root: root.to_path_buf(),
            skills,
        };

        write_lockfile(&lockfile_path, &lockfile).expect("write lockfile");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&lockfile_path).expect("read lockfile"),
        )
        .expect("parse lockfile");
        assert_eq!(value["lockfileVersion"], 4);
        assert_eq!(value["root"], ".");
        assert_eq!(
            value["skills"]["review"]["source"],
            ".sksync/skills/owner/repo/review"
        );
        assert_eq!(
            value["skills"]["review"]["installSource"]["path"],
            "vendor/review"
        );
    }

    #[test]
    fn read_lockfile_rebases_v4_relative_paths_from_lockfile_directory() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let root = temp_dir.path();
        let lockfile_path = root.join("sksync-lock.json");
        std::fs::write(
            &lockfile_path,
            r#"{
              "lockfileVersion": 4,
              "generatedBy": "sksync@test",
              "generatedAt": "unix:1",
              "root": ".",
              "skills": {
                "review": {
                  "source": ".sksync/skills/owner/repo/review",
                  "installSource": {
                    "type": "local",
                    "path": "vendor/review"
                  },
                  "hash": "sha256-review",
                  "files": []
                }
              }
            }"#,
        )
        .expect("write lockfile");

        let lockfile = read_lockfile(&lockfile_path).expect("read lockfile");
        let skill = lockfile
            .skills
            .get(&SkillName::new("review").expect("skill name"))
            .expect("locked skill");

        assert_eq!(lockfile.root, Path::new("."));
        assert_eq!(
            skill.source.as_path(),
            root.join(".sksync/skills/owner/repo/review")
        );
        assert_eq!(
            skill.install_source,
            Some(InstallSource::Local(root.join("vendor/review")))
        );
    }

    #[test]
    fn read_lockfile_keeps_v3_compatibility() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let root = temp_dir.path();
        let lockfile_path = root.join("sksync-lock.json");
        std::fs::write(
            &lockfile_path,
            r#"{
              "lockfileVersion": 3,
              "generatedBy": "sksync@test",
              "generatedAt": "unix:1",
              "root": "/old/machine/project",
              "skills": {
                "review": {
                  "source": ".sksync/skills/review",
                  "installSource": {
                    "type": "git",
                    "url": "https://github.com/owner/repo.git",
                    "ref": "abc123",
                    "path": "skills/review"
                  },
                  "hash": "sha256-review",
                  "files": []
                }
              }
            }"#,
        )
        .expect("write lockfile");

        let lockfile = read_lockfile(&lockfile_path).expect("read v3 lockfile");
        let skill = lockfile
            .skills
            .get(&SkillName::new("review").expect("skill name"))
            .expect("locked skill");

        assert_eq!(lockfile.root, Path::new("/old/machine/project"));
        assert_eq!(skill.source.as_path(), root.join(".sksync/skills/review"));
        assert!(matches!(skill.install_source, Some(InstallSource::Git(_))));
    }

    #[test]
    fn read_lockfile_keeps_v2_targets_compatibility() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let root = temp_dir.path();
        let lockfile_path = root.join("sksync-lock.json");
        std::fs::write(
            &lockfile_path,
            r#"{
              "lockfileVersion": 2,
              "generatedBy": "sksync@test",
              "generatedAt": "unix:1",
              "root": ".",
              "skills": {
                "review": {
                  "source": ".sksync/skills/review",
                  "hash": "sha256-review",
                  "files": [],
                  "targets": [
                    {
                      "agent": "pi",
                      "scope": "project",
                      "path": ".pi/agent/skills/review",
                      "linkType": "symlink"
                    }
                  ]
                }
              }
            }"#,
        )
        .expect("write lockfile");

        let lockfile = read_lockfile(&lockfile_path).expect("read v2 lockfile");
        let skill = lockfile
            .skills
            .get(&SkillName::new("review").expect("skill name"))
            .expect("locked skill");

        assert_eq!(skill.source.as_path(), root.join(".sksync/skills/review"));
        assert_eq!(skill.targets.len(), 1);
        assert_eq!(skill.targets[0].agent, AgentKind::Pi);
    }

    #[test]
    fn write_lockfile_keeps_git_install_source_portable() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let root = temp_dir.path();
        let lockfile_path = root.join("sksync-lock.json");
        let mut skills = BTreeMap::new();
        skills.insert(
            SkillName::new("review").expect("skill name"),
            LockedSkill {
                source: SourcePath::new(root.join(".sksync/skills/owner/repo/review"))
                    .expect("source path"),
                install_source: Some(InstallSource::Git(GitInstallSource {
                    url: "https://github.com/owner/repo.git".to_owned(),
                    reference: Some("abc123".to_owned()),
                    path: PathBuf::from("skills/review"),
                })),
                hash: Digest::new("sha256-review").expect("hash"),
                files: Vec::new(),
                targets: Vec::new(),
            },
        );
        let lockfile = Lockfile {
            generated_by: "sksync@test".to_owned(),
            generated_at: "unix:1".to_owned(),
            root: root.to_path_buf(),
            skills,
        };

        write_lockfile(&lockfile_path, &lockfile).expect("write lockfile");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&lockfile_path).expect("read lockfile"),
        )
        .expect("parse lockfile");
        assert_eq!(
            value["skills"]["review"]["installSource"],
            serde_json::json!({
                "type": "git",
                "url": "https://github.com/owner/repo.git",
                "ref": "abc123",
                "path": "skills/review"
            })
        );
    }

    #[test]
    fn dependency_config_store_merges_agents_for_existing_dependency() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");

        store
            .add_dependency("review", "./review", &["pi".to_owned()])
            .expect("add pi dependency");
        store
            .add_dependency(
                "review",
                "./review",
                &["claude-code".to_owned(), "pi".to_owned()],
            )
            .expect("merge claude-code dependency");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert_eq!(
            value["dependencies"]["review"]["agents"],
            serde_json::json!(["pi", "claude-code"])
        );
    }

    #[test]
    fn dependency_config_store_adds_agents_without_rewriting_source() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "review": {
                  "source": {
                    "provider": "git",
                    "repo": "git@example.com:team/private.git",
                    "ref": "main",
                    "path": "skills/review"
                  },
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");

        let agents = store
            .add_dependency_agents("review", &["claude-code".to_owned()])
            .expect("attach agent");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert_eq!(agents, vec!["pi", "claude-code"]);
        assert!(value["dependencies"]["review"]["source"].is_object());
        assert_eq!(
            value["dependencies"]["review"]["source"]["repo"],
            "git@example.com:team/private.git"
        );
        assert_eq!(
            value["dependencies"]["review"]["agents"],
            serde_json::json!(["pi", "claude-code"])
        );
    }

    #[test]
    fn dependency_config_store_canonicalizes_agent_aliases() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");

        store
            .add_dependency(
                "review",
                "./review",
                &["claude".to_owned(), "claude_code".to_owned()],
            )
            .expect("add dependency");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert_eq!(
            value["dependencies"]["review"]["agents"],
            serde_json::json!(["claude-code"])
        );
    }

    #[test]
    fn file_config_store_rebases_relative_paths_from_config_root() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("nested/sksync.config.json");
        std::fs::create_dir_all(config_path.parent().expect("config parent"))
            .expect("create config parent");
        std::fs::write(
            &config_path,
            r#"{
              "skillDir": "skills",
              "dependencies": {
                "review": {
                  "source": "./vendor/review",
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("write config");

        let config = FileConfigStore::new(&config_path)
            .load_with_default_scope(Scope::Project)
            .expect("config loads");
        let config_root = config_path.parent().expect("config parent");

        assert_eq!(config.skill_dir.as_path(), config_root.join("skills"));
        assert_eq!(
            config.skills[0].source.as_path(),
            config_root.join("skills/review")
        );
        assert_eq!(
            config.skills[0].install_source,
            Some(InstallSource::Local(config_root.join("vendor/review")))
        );
    }

    #[test]
    fn file_config_store_preserves_existing_flat_dependency_source_path() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("nested/sksync.config.json");
        let config_root = config_path.parent().expect("config parent");
        std::fs::create_dir_all(config_root.join("skills/review"))
            .expect("create legacy flat source");
        std::fs::write(
            &config_path,
            r#"{
              "skillDir": "skills",
              "dependencies": {
                "review": {
                  "source": "github:owner/repo/skills/review#main",
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("write config");

        let config = FileConfigStore::new(&config_path)
            .load_with_default_scope(Scope::Project)
            .expect("config loads");

        assert_eq!(
            config.skills[0].source.as_path(),
            config_root.join("skills/review")
        );
    }

    #[test]
    fn file_config_store_uses_namespaced_dependency_source_path_when_flat_path_is_absent() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("nested/sksync.config.json");
        std::fs::create_dir_all(config_path.parent().expect("config parent"))
            .expect("create config parent");
        std::fs::write(
            &config_path,
            r#"{
              "skillDir": "skills",
              "dependencies": {
                "review": {
                  "source": "github:owner/repo/skills/review#main",
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("write config");

        let config = FileConfigStore::new(&config_path)
            .load_with_default_scope(Scope::Project)
            .expect("config loads");
        let config_root = config_path.parent().expect("config parent");

        assert_eq!(
            config.skills[0].source.as_path(),
            config_root.join("skills/owner/repo/review")
        );
    }

    #[test]
    fn dependency_agents_are_deduped_after_alias_resolution() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./.sksync/skills",
              "dependencies": {
                "review": {
                  "source": "./vendor/review",
                  "agents": ["claude", "claude-code", "claude_code"]
                }
              }
            }"#,
        )
        .expect("raw config parses");

        let config = raw
            .resolve_with_default_scope(Scope::Project)
            .expect("config resolves");

        assert_eq!(config.skills[0].agents, vec![AgentKind::ClaudeCode]);
    }

    #[test]
    fn git_shorthand_rejects_parent_directory_subpath() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./.sksync/skills",
              "dependencies": {
                "review": {
                  "source": "owner/repo/../review#main",
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("raw config parses");

        let error = raw
            .resolve_with_default_scope(Scope::Project)
            .expect_err("unsafe git subpath should fail");

        assert!(matches!(
            error,
            ConfigResolveError::InvalidInstallSource { .. }
        ));
    }

    #[test]
    fn structured_git_source_rejects_absolute_subpath() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./.sksync/skills",
              "dependencies": {
                "review": {
                  "source": {
                    "provider": "git",
                    "url": "https://github.com/owner/repo.git",
                    "path": "/tmp/review"
                  },
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("raw config parses");

        let error = raw
            .resolve_with_default_scope(Scope::Project)
            .expect_err("absolute git subpath should fail");

        assert!(matches!(
            error,
            ConfigResolveError::InvalidInstallSource { .. }
        ));
    }

    #[test]
    fn skills_sh_shorthand_source_parses_as_git_source() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./.sksync/skills",
              "dependencies": {
                "review": {
                  "source": "skills.sh/owner/repo/review#main",
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("raw config parses");

        let config = raw
            .resolve_with_default_scope(Scope::Project)
            .expect("config resolves");
        let Some(InstallSource::Git(git)) = &config.skills[0].install_source else {
            panic!("expected git install source");
        };

        assert_eq!(git.url, "https://github.com/owner/repo.git");
        assert_eq!(git.path, Path::new("skills/review"));
        assert_eq!(git.reference.as_deref(), Some("main"));
        assert_eq!(
            config.skills[0].source.as_path(),
            Path::new("./.sksync/skills/owner/repo/review")
        );
    }

    #[test]
    fn github_dependency_uses_source_namespaced_storage_path() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./.sksync/skills",
              "dependencies": {
                "review": {
                  "source": "github:owner/repo/skills/review#main",
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("raw config parses");

        let config = raw
            .resolve_with_default_scope(Scope::Project)
            .expect("config resolves");

        assert_eq!(
            config.skills[0].source.as_path(),
            Path::new("./.sksync/skills/owner/repo/review")
        );
    }

    #[test]
    fn skills_sh_url_source_parses_as_git_source() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "skillDir": "./.sksync/skills",
              "dependencies": {
                "find-skills": {
                  "source": "https://www.skills.sh/vercel-labs/skills/find-skills#main",
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("raw config parses");

        let config = raw
            .resolve_with_default_scope(Scope::Project)
            .expect("config resolves");
        let Some(InstallSource::Git(git)) = &config.skills[0].install_source else {
            panic!("expected git install source");
        };

        assert_eq!(git.url, "https://github.com/vercel-labs/skills.git");
        assert_eq!(git.path, Path::new("skills/find-skills"));
        assert_eq!(git.reference.as_deref(), Some("main"));
        assert_eq!(
            config.skills[0].source.as_path(),
            Path::new("./.sksync/skills/vercel-labs/skills/find-skills")
        );
    }

    #[test]
    fn legacy_agent_mapping_fields_are_supported() {
        let mappings = parse_agent_mapping_config(
            r#"{
              "agents": { "custom-global": { "targetDir": "~/.custom/skills" } },
              "projectAgents": { "custom-project": { "targetDir": ".custom/skills" } }
            }"#,
            "legacy agents.json",
        )
        .expect("legacy mapping parses");

        assert_eq!(
            mappings.global["custom-global"],
            Path::new("~/.custom/skills")
        );
        assert_eq!(
            mappings.project["custom-project"],
            Path::new(".custom/skills")
        );
    }

    #[test]
    fn new_agent_mapping_fields_override_legacy_fields() {
        let mappings = parse_agent_mapping_config(
            r#"{
              "agents": { "pi": { "targetDir": "~/.old-pi/skills" } },
              "global": { "pi": { "targetDir": "~/.new-pi/skills" } },
              "projectAgents": { "pi": { "targetDir": ".old-pi/skills" } },
              "project": { "pi": { "targetDir": ".new-pi/skills" } }
            }"#,
            "mixed agents.json",
        )
        .expect("mixed mapping parses");

        assert_eq!(mappings.global["pi"], Path::new("~/.new-pi/skills"));
        assert_eq!(mappings.project["pi"], Path::new(".new-pi/skills"));
    }

    #[test]
    fn dependency_config_store_removes_selected_agents() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");

        store
            .add_dependency(
                "review",
                "./review",
                &["pi".to_owned(), "claude".to_owned(), "gemini".to_owned()],
            )
            .expect("add dependency");
        let remaining = store
            .remove_dependency_agents("review", &["claude-code".to_owned()])
            .expect("remove selected agent");

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert_eq!(remaining, vec!["pi".to_owned(), "gemini".to_owned()]);
        assert_eq!(
            value["dependencies"]["review"]["agents"],
            serde_json::json!(["pi", "gemini"])
        );
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
        assert!(skill.install_source.is_some());
        assert!(skill.targets.is_empty());
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
                .replace("\"lockfileVersion\": 4", "\"lockfileVersion\": 999"),
        )
        .expect("write lockfile fixture");

        let error = read_lockfile(&lockfile_path).expect_err("unsupported version fails");
        assert!(matches!(
            error,
            LockfileJsonError::UnsupportedVersion {
                found: 999,
                supported: SUPPORTED_LOCKFILE_VERSION,
            }
        ));
    }
}
