use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::application::config::{
    ConfigResolveError, GitInstallSource, InstallSource, ResolvedAgent, ResolvedConfig,
    ResolvedSkill,
};
use crate::application::ports::{
    display_path, ConfigStore, ConfigStoreError, DependencyConfigStore, DependencyConfigStoreError,
    LockfileStore, LockfileStoreError,
};
use crate::application::registry::SourceUrlTransformers;
use crate::domain::agent::AgentKind;
use crate::domain::lockfile::{
    Digest, LinkType, LockedFile, LockedSkill, LockedTarget, Lockfile,
    LEGACY_LOCKFILE_VERSION_WITH_TARGETS, SUPPORTED_LOCKFILE_VERSION,
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
                default_dependency_scope,
            )?;
            if skill_agents.is_empty() {
                return Err(ConfigResolveError::MissingAgents { skill: name });
            }
            let install_source = parse_install_source(&name, raw_dependency.source, config_root)?;

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
            parse_install_source_string(skill, &value, config_root)
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
            Ok(InstallSource::Git(GitInstallSource {
                url: git_url_from_repo(&repo),
                reference: source.reference,
                path,
            }))
        }
    }
}

pub fn parse_install_source_value(
    skill: &str,
    value: &str,
    config_root: Option<&Path>,
) -> Result<InstallSource, ConfigResolveError> {
    parse_install_source_string(skill, value, config_root)
}

fn parse_install_source_string(
    skill: &str,
    value: &str,
    config_root: Option<&Path>,
) -> Result<InstallSource, ConfigResolveError> {
    if value.starts_with("./") || value.starts_with("../") || value.starts_with('/') {
        return Ok(InstallSource::Local(rebase_config_path(
            PathBuf::from(value),
            config_root,
        )));
    }

    let (body, reference) = split_ref(value);
    if let Some(transformed) = SourceUrlTransformers::default().transform_url(body, reference) {
        return Ok(InstallSource::Git(transformed));
    }

    if let Some(parsed) = parse_github_tree_url(value) {
        return Ok(InstallSource::Git(parsed));
    }

    if body.starts_with("registry:") {
        return Err(ConfigResolveError::InvalidInstallSource {
            skill: skill.to_owned(),
            message: "registry sources are not supported; use a provider URL such as https://www.skills.sh/owner/repo/skill-name".to_owned(),
        });
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
    fn try_into_domain(self) -> Result<InstallSource, LockfileJsonError> {
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
            Self::Local { path } => InstallSource::Local(path),
        })
    }

    fn from_domain(source: &InstallSource) -> Self {
        match source {
            InstallSource::Git(git) => Self::Git {
                url: git.url.clone(),
                reference: git.reference.clone(),
                path: git.path.clone(),
            },
            InstallSource::Local(path) => Self::Local { path: path.clone() },
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

    if found != SUPPORTED_LOCKFILE_VERSION && found != LEGACY_LOCKFILE_VERSION_WITH_TARGETS {
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
        if self.lockfile_version != SUPPORTED_LOCKFILE_VERSION
            && self.lockfile_version != LEGACY_LOCKFILE_VERSION_WITH_TARGETS
        {
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
                    install_source: raw_skill
                        .install_source
                        .map(RawLockedInstallSource::try_into_domain)
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
                        install_source: skill
                            .install_source
                            .as_ref()
                            .map(RawLockedInstallSource::from_domain),
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
            root: lockfile.root.clone(),
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
    use crate::application::config::{ConfigResolveError, InstallSource};
    use crate::application::ports::{ConfigStore, DependencyConfigStore};
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
            Some(crate::application::config::InstallSource::Local(
                config_root.join("vendor/review")
            ))
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
                .replace("\"lockfileVersion\": 3", "\"lockfileVersion\": 999"),
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
