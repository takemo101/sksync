use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::application::bundle::{
    git_source_to_config_string, BundleAddPlan, BundleAddPlanItem, BundleAddStatus,
    BundleRemovePlan, BundleRemovePlanItem, BundleRemoveStatus, BundleSyncPlan, BundleSyncPlanItem,
    BundleSyncSourceResolution, BundleSyncStatus, LoadedBundleEntry,
};
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
use crate::domain::bundle::{BundleEntry, BundleManifest, BundleName, BundleNameError};
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
    pub default_agents: Vec<String>,
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
    #[serde(default)]
    pub bundles: Vec<RawBundleProvenance>,
    #[serde(default)]
    pub managed_by_bundles: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct RawBundleProvenance {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawBundleManifest {
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    pub name: String,
    pub description: String,
    pub entries: BTreeMap<String, RawBundleEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawBundleEntry {
    pub source: String,
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
pub struct BundleExportDependencyConfig {
    pub name: String,
    pub source: String,
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

#[derive(Debug, Error)]
pub enum BundleManifestJsonError {
    #[error("failed to read bundle manifest at {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create bundle manifest directory {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse bundle manifest at {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize bundle manifest at {path}: {source}")]
    Serialize {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write bundle manifest at {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid bundle name '{name}': {source}")]
    InvalidName {
        name: String,
        #[source]
        source: BundleNameError,
    },
    #[error("bundle description must not be empty")]
    EmptyDescription,
    #[error("bundle entries must not be empty")]
    EmptyEntries,
    #[error("invalid bundle entry skill name '{name}': {source}")]
    InvalidEntryName {
        name: String,
        #[source]
        source: crate::domain::skill::SkillNameError,
    },
    #[error("bundle entry '{name}' duplicates another entry after name normalization")]
    DuplicateEntryName { name: String },
    #[error("bundle entry '{name}' source must not be empty")]
    EmptyEntrySource { name: String },
}

pub fn read_bundle_manifest(
    path: impl AsRef<Path>,
) -> Result<BundleManifest, BundleManifestJsonError> {
    let path = path.as_ref();
    let content =
        std::fs::read_to_string(path).map_err(|source| BundleManifestJsonError::Read {
            path: display_path(path),
            source,
        })?;
    parse_bundle_manifest(&content, &display_path(path))
}

pub fn write_bundle_manifest(
    path: impl AsRef<Path>,
    manifest: &BundleManifest,
) -> Result<(), BundleManifestJsonError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| BundleManifestJsonError::CreateDir {
            path: display_path(parent),
            source,
        })?;
    }
    let entries = manifest
        .entries
        .iter()
        .map(|entry| {
            (
                entry.skill_name.as_str().to_owned(),
                RawBundleEntry {
                    source: entry.source.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let raw = RawBundleManifest {
        schema: Some(
            "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json"
                .to_owned(),
        ),
        name: manifest.name.as_str().to_owned(),
        description: manifest.description.clone(),
        entries,
    };
    let content = serde_json::to_string_pretty(&raw).map_err(|source| {
        BundleManifestJsonError::Serialize {
            path: display_path(path),
            source,
        }
    })?;
    std::fs::write(path, format!("{content}\n")).map_err(|source| BundleManifestJsonError::Write {
        path: display_path(path),
        source,
    })
}

pub fn parse_bundle_manifest(
    content: &str,
    path: &str,
) -> Result<BundleManifest, BundleManifestJsonError> {
    let raw = serde_json::from_str::<RawBundleManifest>(content).map_err(|source| {
        BundleManifestJsonError::Parse {
            path: path.to_owned(),
            source,
        }
    })?;
    let name = BundleName::new(raw.name.clone()).map_err(|source| {
        BundleManifestJsonError::InvalidName {
            name: raw.name,
            source,
        }
    })?;
    if raw.description.trim().is_empty() {
        return Err(BundleManifestJsonError::EmptyDescription);
    }
    if raw.entries.is_empty() {
        return Err(BundleManifestJsonError::EmptyEntries);
    }

    let mut entries = Vec::with_capacity(raw.entries.len());
    let mut seen_entries = BTreeSet::new();
    for (entry_name, entry) in raw.entries {
        let skill_name = SkillName::new(entry_name.clone()).map_err(|source| {
            BundleManifestJsonError::InvalidEntryName {
                name: entry_name.clone(),
                source,
            }
        })?;
        if !seen_entries.insert(skill_name.clone()) {
            return Err(BundleManifestJsonError::DuplicateEntryName { name: entry_name });
        }
        if entry.source.trim().is_empty() {
            return Err(BundleManifestJsonError::EmptyEntrySource { name: entry_name });
        }
        entries.push(BundleEntry {
            skill_name,
            source: entry.source,
        });
    }

    Ok(BundleManifest {
        name,
        description: raw.description,
        entries,
    })
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

        let default_agents = parse_default_agents(self.default_agents)?;
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
            default_agents,
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

fn parse_default_agents(agents: Vec<String>) -> Result<Vec<AgentKind>, ConfigResolveError> {
    let mut default_agents = Vec::with_capacity(agents.len());
    let mut seen_agents = BTreeSet::new();
    for agent in agents {
        let kind = parse_agent_kind(&agent)?;
        let key = kind.as_str().to_owned();
        if seen_agents.insert(key) {
            default_agents.push(kind);
        }
    }
    Ok(default_agents)
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

    pub fn load_bundle_export_dependencies(
        &self,
    ) -> Result<Vec<BundleExportDependencyConfig>, DependencyConfigStoreError> {
        let value = self.load_or_default()?;
        let Some(dependencies) = value.get("dependencies") else {
            return Ok(Vec::new());
        };
        let dependencies = dependencies.as_object().ok_or_else(|| {
            DependencyConfigStoreError::InvalidField("dependencies must be an object".to_owned())
        })?;
        let mut exported = Vec::with_capacity(dependencies.len());
        for (name, dependency) in dependencies {
            SkillName::new(name.clone()).map_err(|source| {
                DependencyConfigStoreError::InvalidField(format!(
                    "invalid dependency name '{name}': {source}"
                ))
            })?;
            exported.push(BundleExportDependencyConfig {
                name: name.clone(),
                source: dependency_export_source(dependency, name)?,
            });
        }
        exported.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(exported)
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
        let mut dependency = if dependencies
            .get(skill_name)
            .and_then(|dependency| dependency_source_matches(dependency, skill_name, source).ok())
            .unwrap_or(false)
        {
            dependencies
                .get(skill_name)
                .cloned()
                .unwrap_or_else(|| json!({}))
        } else {
            json!({})
        };
        let object = dependency.as_object_mut().ok_or_else(|| {
            DependencyConfigStoreError::InvalidField(format!(
                "dependencies.{skill_name} must be an object"
            ))
        })?;
        object.insert("source".to_owned(), json!(source));
        object.insert("agents".to_owned(), json!(merged_agents));
        dependencies.insert(skill_name.to_owned(), dependency);
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
    pub fn plan_bundle_add(
        &self,
        entries: &[LoadedBundleEntry],
        agents: &[String],
        provenance: &crate::domain::bundle::BundleProvenance,
    ) -> Result<BundleAddPlan, DependencyConfigStoreError> {
        let value = self.load_or_default()?;
        let empty_dependencies = serde_json::Map::new();
        let dependencies = value
            .get("dependencies")
            .and_then(|dependencies| dependencies.as_object())
            .unwrap_or(&empty_dependencies);
        let agents = normalize_agent_names(agents);
        let items = entries
            .iter()
            .map(|entry| {
                let existing = dependencies.get(&entry.skill_name);
                let status = match existing {
                    None => BundleAddStatus::Create,
                    Some(existing) => {
                        if !dependency_source_matches(
                            existing,
                            &entry.skill_name,
                            &entry.normalized_source,
                        )? {
                            BundleAddStatus::Conflict
                        } else {
                            let existing_agents =
                                dependency_agents(Some(existing), &entry.skill_name)?;
                            let existing_bundles = dependency_bundles(existing, &entry.skill_name)?;
                            let has_all_agents = agents.iter().all(|agent| {
                                existing_agents
                                    .iter()
                                    .map(|existing| normalize_agent_name(existing))
                                    .any(|existing| existing == *agent)
                            });
                            let has_provenance = existing_bundles.iter().any(|bundle| {
                                bundle.name == provenance.name.as_str()
                                    && bundle.source == provenance.source
                            });
                            if has_all_agents && has_provenance {
                                BundleAddStatus::Skipped
                            } else {
                                BundleAddStatus::Merge
                            }
                        }
                    }
                };
                let message = if status == BundleAddStatus::Conflict {
                    existing
                        .and_then(|existing| dependency_source(existing, &entry.skill_name).ok())
                        .map(|source| {
                            format!(
                            "existing dependency source {source:?} differs from bundle source {:?}",
                            entry.normalized_source
                        )
                        })
                } else {
                    None
                };
                Ok(BundleAddPlanItem {
                    skill_name: entry.skill_name.clone(),
                    source: entry.normalized_source.clone(),
                    agents: agents.clone(),
                    provenance: provenance.clone(),
                    status,
                    message,
                })
            })
            .collect::<Result<Vec<_>, DependencyConfigStoreError>>()?;
        Ok(BundleAddPlan { items })
    }

    pub fn apply_bundle_add(&self, plan: &BundleAddPlan) -> Result<(), DependencyConfigStoreError> {
        if plan.has_conflicts() {
            return Err(DependencyConfigStoreError::InvalidField(
                "bundle add plan contains conflicts".to_owned(),
            ));
        }
        let mut value = self.load_or_default()?;
        let dependencies = dependencies_object_mut(&mut value)?;
        for item in &plan.items {
            if item.status == BundleAddStatus::Skipped {
                continue;
            }
            let existing = dependencies.get_mut(&item.skill_name);
            match existing {
                Some(dependency) => merge_bundle_dependency(dependency, item)?,
                None => {
                    dependencies.insert(
                        item.skill_name.clone(),
                        json!({
                            "source": item.source,
                            "agents": item.agents,
                            "bundles": [{
                                "name": item.provenance.name.as_str(),
                                "source": item.provenance.source,
                            }],
                            "managedByBundles": true,
                        }),
                    );
                }
            }
        }
        self.write_value(&value)
    }

    pub fn plan_bundle_remove(
        &self,
        bundle_name: &crate::domain::bundle::BundleName,
        source: Option<&str>,
    ) -> Result<BundleRemovePlan, DependencyConfigStoreError> {
        let value = self.load_or_default()?;
        let empty_dependencies = serde_json::Map::new();
        let dependencies = value
            .get("dependencies")
            .and_then(|dependencies| dependencies.as_object())
            .unwrap_or(&empty_dependencies);
        let mut matching_sources = BTreeSet::new();
        for (skill_name, dependency) in dependencies {
            for bundle in dependency_bundles(dependency, skill_name)? {
                if bundle.name == bundle_name.as_str() {
                    matching_sources.insert(bundle.source);
                }
            }
        }
        let ambiguous_sources = if source.is_none() && matching_sources.len() > 1 {
            matching_sources.iter().cloned().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if !ambiguous_sources.is_empty() {
            return Ok(BundleRemovePlan {
                bundle: bundle_name.clone(),
                source: source.map(str::to_owned),
                items: vec![BundleRemovePlanItem {
                    skill_name: bundle_name.as_str().to_owned(),
                    status: BundleRemoveStatus::Ambiguous,
                    source: None,
                    message: Some(format!(
                        "bundle name matches multiple sources: {}",
                        ambiguous_sources.join(", ")
                    )),
                }],
                ambiguous_sources,
            });
        }

        let mut items = Vec::new();
        for (skill_name, dependency) in dependencies {
            let bundles = dependency_bundles(dependency, skill_name)?;
            let matching = bundles.iter().any(|bundle| {
                bundle.name == bundle_name.as_str()
                    && source.is_none_or(|source| bundle.source == source)
            });
            if !matching {
                continue;
            }
            let remaining = bundles
                .iter()
                .filter(|bundle| {
                    !(bundle.name == bundle_name.as_str()
                        && source.is_none_or(|source| bundle.source == source))
                })
                .count();
            let managed = dependency_managed_by_bundles(dependency, skill_name)?;
            let status = if remaining == 0 && managed {
                BundleRemoveStatus::Remove
            } else {
                BundleRemoveStatus::DetachProvenance
            };
            items.push(BundleRemovePlanItem {
                skill_name: skill_name.clone(),
                status,
                source: source.map(str::to_owned),
                message: None,
            });
        }

        if items.is_empty() {
            items.push(BundleRemovePlanItem {
                skill_name: bundle_name.as_str().to_owned(),
                status: BundleRemoveStatus::NotFound,
                source: source.map(str::to_owned),
                message: Some("no dependency has matching bundle provenance".to_owned()),
            });
        }

        Ok(BundleRemovePlan {
            bundle: bundle_name.clone(),
            source: source.map(str::to_owned),
            items,
            ambiguous_sources,
        })
    }

    pub fn resolve_bundle_sync_source(
        &self,
        bundle_name: &crate::domain::bundle::BundleName,
        source: Option<&str>,
    ) -> Result<BundleSyncSourceResolution, DependencyConfigStoreError> {
        let value = self.load_or_default()?;
        let empty_dependencies = serde_json::Map::new();
        let dependencies = value
            .get("dependencies")
            .and_then(|dependencies| dependencies.as_object())
            .unwrap_or(&empty_dependencies);
        let mut matching_sources = BTreeSet::new();
        for (skill_name, dependency) in dependencies {
            for bundle in dependency_bundles(dependency, skill_name)? {
                if bundle.name == bundle_name.as_str() {
                    matching_sources.insert(bundle.source);
                }
            }
        }
        if let Some(source) = source {
            if matching_sources.iter().any(|existing| existing == source) {
                return Ok(BundleSyncSourceResolution::Resolved(source.to_owned()));
            }
            return Ok(BundleSyncSourceResolution::NotFound);
        }
        match matching_sources.len() {
            0 => Ok(BundleSyncSourceResolution::NotFound),
            1 => Ok(BundleSyncSourceResolution::Resolved(
                matching_sources.into_iter().next().unwrap_or_default(),
            )),
            _ => Ok(BundleSyncSourceResolution::Ambiguous(
                matching_sources.into_iter().collect(),
            )),
        }
    }

    pub fn plan_bundle_sync(
        &self,
        provenance: &crate::domain::bundle::BundleProvenance,
        entries: &[LoadedBundleEntry],
        fallback_agents: &[String],
    ) -> Result<BundleSyncPlan, DependencyConfigStoreError> {
        let value = self.load_or_default()?;
        let empty_dependencies = serde_json::Map::new();
        let dependencies = value
            .get("dependencies")
            .and_then(|dependencies| dependencies.as_object())
            .unwrap_or(&empty_dependencies);
        let inferred_agents = infer_bundle_agents(dependencies, provenance)?;
        let fallback_agents = normalize_agent_names(fallback_agents);
        let agents = if inferred_agents.is_empty() {
            fallback_agents
        } else {
            inferred_agents
        };
        let manifest_entries = entries
            .iter()
            .map(|entry| (entry.skill_name.as_str(), entry))
            .collect::<BTreeMap<_, _>>();
        let mut items = Vec::new();
        let mut keep_count = 0;

        for entry in entries {
            match dependencies.get(&entry.skill_name) {
                Some(existing)
                    if dependency_has_bundle(existing, &entry.skill_name, provenance)? =>
                {
                    if dependency_source_matches(
                        existing,
                        &entry.skill_name,
                        &entry.normalized_source,
                    )? {
                        keep_count += 1;
                    } else {
                        let local_source = dependency_source(existing, &entry.skill_name).ok();
                        items.push(BundleSyncPlanItem {
                            skill_name: entry.skill_name.clone(),
                            status: BundleSyncStatus::SourceChanged,
                            local_source: local_source.clone(),
                            manifest_source: Some(entry.normalized_source.clone()),
                            agents: Vec::new(),
                            message: Some(format!(
                                "local source {:?} differs from bundle manifest source {:?}",
                                local_source.unwrap_or_else(|| "<unknown>".to_owned()),
                                entry.normalized_source
                            )),
                        });
                    }
                }
                Some(existing) => {
                    if dependency_source_matches(
                        existing,
                        &entry.skill_name,
                        &entry.normalized_source,
                    )? {
                        items.push(BundleSyncPlanItem {
                            skill_name: entry.skill_name.clone(),
                            status: BundleSyncStatus::Adopt,
                            local_source: dependency_source(existing, &entry.skill_name).ok(),
                            manifest_source: Some(entry.normalized_source.clone()),
                            agents: agents.clone(),
                            message: None,
                        });
                    } else {
                        let local_source = dependency_source(existing, &entry.skill_name).ok();
                        items.push(BundleSyncPlanItem {
                            skill_name: entry.skill_name.clone(),
                            status: BundleSyncStatus::SourceChanged,
                            local_source: local_source.clone(),
                            manifest_source: Some(entry.normalized_source.clone()),
                            agents: Vec::new(),
                            message: Some(format!(
                                "existing dependency source {:?} differs from bundle manifest source {:?}",
                                local_source.unwrap_or_else(|| "<unknown>".to_owned()),
                                entry.normalized_source
                            )),
                        });
                    }
                }
                None if agents.is_empty() => items.push(BundleSyncPlanItem {
                    skill_name: entry.skill_name.clone(),
                    status: BundleSyncStatus::MissingAgents,
                    local_source: None,
                    manifest_source: Some(entry.normalized_source.clone()),
                    agents: Vec::new(),
                    message: Some(
                        "no dependency agents could be inferred; pass --agent".to_owned(),
                    ),
                }),
                None => items.push(BundleSyncPlanItem {
                    skill_name: entry.skill_name.clone(),
                    status: BundleSyncStatus::Add,
                    local_source: None,
                    manifest_source: Some(entry.normalized_source.clone()),
                    agents: agents.clone(),
                    message: None,
                }),
            }
        }

        for (skill_name, dependency) in dependencies {
            if !dependency_has_bundle(dependency, skill_name, provenance)? {
                continue;
            }
            if manifest_entries.contains_key(skill_name.as_str()) {
                continue;
            }
            let bundles = dependency_bundles(dependency, skill_name)?;
            let remaining = bundles
                .iter()
                .filter(|bundle| {
                    !(bundle.name == provenance.name.as_str() && bundle.source == provenance.source)
                })
                .count();
            let managed = dependency_managed_by_bundles(dependency, skill_name)?;
            let status = if remaining == 0 && managed {
                BundleSyncStatus::Remove
            } else {
                BundleSyncStatus::DetachProvenance
            };
            items.push(BundleSyncPlanItem {
                skill_name: skill_name.clone(),
                status,
                local_source: dependency_source(dependency, skill_name).ok(),
                manifest_source: None,
                agents: Vec::new(),
                message: None,
            });
        }

        Ok(BundleSyncPlan {
            bundle: provenance.name.clone(),
            source: provenance.source.clone(),
            items,
            keep_count,
        })
    }

    pub fn detach_bundle_provenance(
        &self,
        plan: &BundleRemovePlan,
    ) -> Result<(), DependencyConfigStoreError> {
        if plan.is_ambiguous() {
            return Err(DependencyConfigStoreError::InvalidField(
                "bundle remove plan is ambiguous".to_owned(),
            ));
        }
        let mut value = self.load_or_default()?;
        let dependencies = dependencies_object_mut(&mut value)?;
        for item in &plan.items {
            if item.status != BundleRemoveStatus::DetachProvenance {
                continue;
            }
            let Some(dependency) = dependencies.get_mut(&item.skill_name) else {
                continue;
            };
            remove_bundle_from_dependency(
                dependency,
                &plan.bundle,
                plan.source.as_deref(),
                &item.skill_name,
            )?;
        }
        self.write_value(&value)
    }

    fn load_or_default(&self) -> Result<serde_json::Value, DependencyConfigStoreError> {
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
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| {
                DependencyConfigStoreError::CreateDir {
                    path: display_path(parent),
                    source,
                }
            })?;
        }
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

fn dependencies_object(
    value: &serde_json::Value,
) -> Result<&serde_json::Map<String, serde_json::Value>, DependencyConfigStoreError> {
    let Some(dependencies) = value.get("dependencies") else {
        return Err(DependencyConfigStoreError::InvalidField(
            "dependencies must be an object".to_owned(),
        ));
    };
    dependencies.as_object().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField("dependencies must be an object".to_owned())
    })
}

fn normalize_agent_names(agents: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for agent in agents {
        let agent = normalize_agent_name(agent);
        if seen.insert(agent.clone()) {
            normalized.push(agent);
        }
    }
    normalized
}

fn normalize_agent_name(agent: &str) -> String {
    AgentKind::from_str(agent)
        .map(|agent| agent.as_str().to_owned())
        .unwrap_or_else(|_| agent.trim().to_ascii_lowercase())
}

fn dependency_export_source(
    dependency: &serde_json::Value,
    skill_name: &str,
) -> Result<String, DependencyConfigStoreError> {
    let dependency = dependency.as_object().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name} must be an object"
        ))
    })?;
    let source = dependency.get("source").ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.source is required"
        ))
    })?;
    if let Some(source) = source.as_str() {
        parse_source_string(source).map_err(|error| {
            DependencyConfigStoreError::InvalidField(format!(
                "dependencies.{skill_name}.source is invalid: {error}"
            ))
        })?;
        return Ok(source.to_owned());
    }
    let raw = serde_json::from_value::<RawInstallSource>(source.clone()).map_err(|error| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.source is invalid: {error}"
        ))
    })?;
    let install_source = parse_install_source(skill_name, raw, None).map_err(|error| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.source is invalid: {error}"
        ))
    })?;
    Ok(match install_source {
        InstallSource::Git(git) => git_source_to_config_string(&git),
        InstallSource::Local(path) => path.to_string_lossy().replace('\\', "/"),
    })
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

fn dependency_source(
    dependency: &serde_json::Value,
    skill_name: &str,
) -> Result<String, DependencyConfigStoreError> {
    let dependency = dependency.as_object().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name} must be an object"
        ))
    })?;
    let source = dependency.get("source").ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.source is required"
        ))
    })?;
    if let Some(source) = source.as_str() {
        Ok(source.to_owned())
    } else {
        serde_json::to_string(source).map_err(DependencyConfigStoreError::Serialize)
    }
}

fn dependency_source_matches(
    dependency: &serde_json::Value,
    skill_name: &str,
    candidate: &str,
) -> Result<bool, DependencyConfigStoreError> {
    let dependency = dependency.as_object().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name} must be an object"
        ))
    })?;
    let source = dependency.get("source").ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.source is required"
        ))
    })?;
    if source.as_str() == Some(candidate) {
        return Ok(true);
    }
    let candidate = match parse_source_string(candidate) {
        Ok(candidate) => candidate,
        Err(_) => return Ok(false),
    };
    let existing = if let Some(existing) = source.as_str() {
        parse_source_string(existing).ok()
    } else {
        serde_json::from_value::<RawInstallSource>(source.clone())
            .ok()
            .and_then(|raw| parse_install_source(skill_name, raw, None).ok())
    };
    Ok(existing.is_some_and(|existing| install_sources_equivalent(&existing, &candidate)))
}

fn install_sources_equivalent(left: &InstallSource, right: &InstallSource) -> bool {
    match (left, right) {
        (InstallSource::Git(left), InstallSource::Git(right)) => {
            left.url == right.url
                && left.path == right.path
                && git_references_equivalent(left.reference.as_deref(), right.reference.as_deref())
        }
        (InstallSource::Local(left), InstallSource::Local(right)) => left == right,
        _ => false,
    }
}

fn git_references_equivalent(left: Option<&str>, right: Option<&str>) -> bool {
    let left = left.unwrap_or("HEAD");
    let right = right.unwrap_or("HEAD");
    left == right
}

fn dependency_bundles(
    dependency: &serde_json::Value,
    skill_name: &str,
) -> Result<Vec<RawBundleProvenance>, DependencyConfigStoreError> {
    let dependency = dependency.as_object().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name} must be an object"
        ))
    })?;
    let Some(bundles) = dependency.get("bundles") else {
        return Ok(Vec::new());
    };
    serde_json::from_value(bundles.clone()).map_err(|error| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name}.bundles is invalid: {error}"
        ))
    })
}

fn dependency_has_bundle(
    dependency: &serde_json::Value,
    skill_name: &str,
    provenance: &crate::domain::bundle::BundleProvenance,
) -> Result<bool, DependencyConfigStoreError> {
    Ok(dependency_bundles(dependency, skill_name)?
        .iter()
        .any(|bundle| {
            bundle.name == provenance.name.as_str() && bundle.source == provenance.source
        }))
}

fn infer_bundle_agents(
    dependencies: &serde_json::Map<String, serde_json::Value>,
    provenance: &crate::domain::bundle::BundleProvenance,
) -> Result<Vec<String>, DependencyConfigStoreError> {
    let mut agents = Vec::new();
    let mut seen = BTreeSet::new();
    for (skill_name, dependency) in dependencies {
        if !dependency_has_bundle(dependency, skill_name, provenance)? {
            continue;
        }
        for agent in dependency_agents(Some(dependency), skill_name)? {
            let normalized = normalize_agent_name(&agent);
            if seen.insert(normalized.clone()) {
                agents.push(normalized);
            }
        }
    }
    Ok(agents)
}

fn dependency_managed_by_bundles(
    dependency: &serde_json::Value,
    skill_name: &str,
) -> Result<bool, DependencyConfigStoreError> {
    let dependency = dependency.as_object().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name} must be an object"
        ))
    })?;
    match dependency.get("managedByBundles") {
        Some(value) => value.as_bool().ok_or_else(|| {
            DependencyConfigStoreError::InvalidField(format!(
                "dependencies.{skill_name}.managedByBundles must be a boolean"
            ))
        }),
        None => Ok(false),
    }
}

fn merge_bundle_dependency(
    dependency: &mut serde_json::Value,
    item: &BundleAddPlanItem,
) -> Result<(), DependencyConfigStoreError> {
    let object = dependency.as_object_mut().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{} must be an object",
            item.skill_name
        ))
    })?;
    let mut merged_agents = dependency_agents(
        Some(&serde_json::Value::Object(object.clone())),
        &item.skill_name,
    )?;
    let mut seen_agents = merged_agents
        .iter()
        .map(|agent| normalize_agent_name(agent))
        .collect::<BTreeSet<_>>();
    for agent in &item.agents {
        if seen_agents.insert(agent.clone()) {
            merged_agents.push(agent.clone());
        }
    }
    object.insert("agents".to_owned(), json!(merged_agents));

    let bundles_value = object.entry("bundles").or_insert_with(|| json!([]));
    let mut bundles = serde_json::from_value::<Vec<RawBundleProvenance>>(bundles_value.clone())
        .map_err(|error| {
            DependencyConfigStoreError::InvalidField(format!(
                "dependencies.{}.bundles is invalid: {error}",
                item.skill_name
            ))
        })?;
    let provenance = RawBundleProvenance {
        name: item.provenance.name.as_str().to_owned(),
        source: item.provenance.source.clone(),
    };
    if !bundles.iter().any(|bundle| bundle == &provenance) {
        bundles.push(provenance);
    }
    object.insert("bundles".to_owned(), json!(bundles));
    Ok(())
}

fn remove_bundle_from_dependency(
    dependency: &mut serde_json::Value,
    bundle_name: &BundleName,
    source: Option<&str>,
    skill_name: &str,
) -> Result<(), DependencyConfigStoreError> {
    let object = dependency.as_object_mut().ok_or_else(|| {
        DependencyConfigStoreError::InvalidField(format!(
            "dependencies.{skill_name} must be an object"
        ))
    })?;
    let bundles_value = object.get("bundles").cloned().unwrap_or_else(|| json!([]));
    let mut bundles =
        serde_json::from_value::<Vec<RawBundleProvenance>>(bundles_value).map_err(|error| {
            DependencyConfigStoreError::InvalidField(format!(
                "dependencies.{skill_name}.bundles is invalid: {error}"
            ))
        })?;
    bundles.retain(|bundle| {
        !(bundle.name == bundle_name.as_str()
            && source.is_none_or(|source| bundle.source == source))
    });
    if bundles.is_empty() {
        object.remove("bundles");
    } else {
        object.insert("bundles".to_owned(), json!(bundles));
    }
    Ok(())
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
        parse_agent_mapping_config, parse_bundle_manifest, read_lockfile, write_bundle_manifest,
        write_lockfile, BundleManifestJsonError, FileConfigStore, FileDependencyConfigStore,
        LockfileJsonError, RawConfig,
    };
    use crate::application::config::ConfigResolveError;
    use crate::application::ports::{ConfigStore, DependencyConfigStore};
    use crate::domain::agent::AgentKind;
    use crate::domain::bundle::{BundleEntry, BundleManifest, BundleName};
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
        assert_eq!(
            config.default_agents,
            vec![AgentKind::Custom(
                crate::domain::agent::AgentName::new("universal").unwrap()
            )]
        );
    }

    #[test]
    fn parses_default_agents_with_alias_deduplication() {
        let raw = serde_json::from_str::<RawConfig>(
            r#"{
              "defaultAgents": ["universal", "claude", "claude_code", "pi"]
            }"#,
        )
        .expect("raw config parses");

        let config = raw.resolve().expect("config resolves");

        assert_eq!(
            config
                .default_agents
                .iter()
                .map(AgentKind::as_str)
                .collect::<Vec<_>>(),
            vec!["universal", "claude-code", "pi"]
        );
    }

    #[test]
    fn bundle_manifest_parses_valid_manifest() {
        let manifest = parse_bundle_manifest(
            r#"{
              "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json",
              "name": "review-workflow",
              "description": "Review workflow skills.",
              "entries": {
                "review": { "source": "./skills/review" },
                "qa": { "source": "github:org/qa-skills/skills/qa#main" }
              }
            }"#,
            "inline",
        )
        .expect("manifest parses");

        assert_eq!(manifest.name.as_str(), "review-workflow");
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(manifest.entries[0].skill_name.as_str(), "qa");
        assert_eq!(manifest.entries[1].skill_name.as_str(), "review");
    }

    #[test]
    fn write_bundle_manifest_outputs_schema_and_sorted_entries() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("sksync.bundle.json");
        let manifest = BundleManifest {
            name: BundleName::new("team-baseline").unwrap(),
            description: "Exported from sksync project config.".to_owned(),
            entries: vec![
                BundleEntry {
                    skill_name: SkillName::new("review").unwrap(),
                    source: "./skills/review".to_owned(),
                },
                BundleEntry {
                    skill_name: SkillName::new("qa").unwrap(),
                    source: "./skills/qa".to_owned(),
                },
            ],
        };

        write_bundle_manifest(&path, &manifest).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let value = serde_json::from_str::<serde_json::Value>(&content).unwrap();
        assert_eq!(
            value["$schema"],
            "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.bundle.schema.json"
        );
        assert_eq!(
            value["entries"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["qa".to_owned(), "review".to_owned()]
        );
        parse_bundle_manifest(&content, path.to_string_lossy().as_ref())
            .expect("written manifest should parse");
    }

    #[test]
    fn bundle_manifest_rejects_unknown_top_level_field() {
        assert!(matches!(
            parse_bundle_manifest(
                r#"{
                  "name": "review-workflow",
                  "description": "Review workflow skills.",
                  "entries": { "review": { "source": "./skills/review" } },
                  "unexpected": true
                }"#,
                "inline"
            ),
            Err(BundleManifestJsonError::Parse { .. })
        ));
    }

    #[test]
    fn bundle_manifest_rejects_agents_in_entry() {
        assert!(matches!(
            parse_bundle_manifest(
                r#"{
                  "name": "review-workflow",
                  "description": "Review workflow skills.",
                  "entries": { "review": { "source": "./skills/review", "agents": ["pi"] } }
                }"#,
                "inline"
            ),
            Err(BundleManifestJsonError::Parse { .. })
        ));
    }

    #[test]
    fn bundle_manifest_rejects_empty_entries() {
        assert!(matches!(
            parse_bundle_manifest(
                r#"{
                  "name": "review-workflow",
                  "description": "Review workflow skills.",
                  "entries": {}
                }"#,
                "inline"
            ),
            Err(BundleManifestJsonError::EmptyEntries)
        ));
    }

    #[test]
    fn bundle_manifest_rejects_invalid_entry_name() {
        assert!(matches!(
            parse_bundle_manifest(
                r#"{
                  "name": "review-workflow",
                  "description": "Review workflow skills.",
                  "entries": { "team/review": { "source": "./skills/review" } }
                }"#,
                "inline"
            ),
            Err(BundleManifestJsonError::InvalidEntryName { .. })
        ));
    }

    #[test]
    fn bundle_manifest_rejects_empty_entry_source() {
        assert!(matches!(
            parse_bundle_manifest(
                r#"{
                  "name": "review-workflow",
                  "description": "Review workflow skills.",
                  "entries": { "review": { "source": "  " } }
                }"#,
                "inline"
            ),
            Err(BundleManifestJsonError::EmptyEntrySource { .. })
        ));
    }

    #[test]
    fn bundle_manifest_rejects_duplicate_entry_after_name_trimming() {
        assert!(matches!(
            parse_bundle_manifest(
                r#"{
                  "name": "review-workflow",
                  "description": "Review workflow skills.",
                  "entries": {
                    "review": { "source": "./skills/review" },
                    " review ": { "source": "./skills/other-review" }
                  }
                }"#,
                "inline"
            ),
            Err(BundleManifestJsonError::DuplicateEntryName { .. })
        ));
    }

    #[test]
    fn bundle_export_dependencies_preserve_shorthand_sources_and_ignore_provenance() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "review": {
                  "source": "github:org/repo/skills/review#main",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundle" }],
                  "managedByBundles": true
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");

        let dependencies = store.load_bundle_export_dependencies().unwrap();

        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].name, "review");
        assert_eq!(dependencies[0].source, "github:org/repo/skills/review#main");
    }

    #[test]
    fn bundle_export_dependencies_convert_structured_git_sources_to_tree_urls() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "review": {
                  "source": {
                    "provider": "git",
                    "repo": "org/repo",
                    "path": "skills/review",
                    "ref": "main"
                  },
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");

        let dependencies = store.load_bundle_export_dependencies().unwrap();

        assert_eq!(
            dependencies[0].source,
            "https://github.com/org/repo/tree/main/skills/review"
        );
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
    fn bundle_add_plan_classifies_create_merge_conflict_and_skipped() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "merge-me": {
                  "source": {
                    "provider": "git",
                    "repo": "org/repo",
                    "path": "skills/merge-me"
                  },
                  "agents": ["pi"]
                },
                "skip-me": {
                  "source": "./skip",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/baseline" }],
                  "managedByBundles": true
                },
                "conflict-me": {
                  "source": "./other",
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");
        let provenance = crate::domain::bundle::BundleProvenance {
            name: crate::domain::bundle::BundleName::new("baseline").unwrap(),
            source: "./bundles/baseline".to_owned(),
        };
        let entries = vec![
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "new-skill".to_owned(),
                original_source: "./new".to_owned(),
                normalized_source: "./new".to_owned(),
            },
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "merge-me".to_owned(),
                original_source: "github:org/repo/skills/merge-me".to_owned(),
                normalized_source: "https://github.com/org/repo/tree/HEAD/skills/merge-me"
                    .to_owned(),
            },
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "skip-me".to_owned(),
                original_source: "./skip".to_owned(),
                normalized_source: "./skip".to_owned(),
            },
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "conflict-me".to_owned(),
                original_source: "./same".to_owned(),
                normalized_source: "./same".to_owned(),
            },
        ];

        let plan = store
            .plan_bundle_add(
                &entries,
                &["pi".to_owned(), "claude".to_owned()],
                &provenance,
            )
            .unwrap();

        assert_eq!(
            plan.items[0].status,
            crate::application::bundle::BundleAddStatus::Create
        );
        assert_eq!(
            plan.items[1].status,
            crate::application::bundle::BundleAddStatus::Merge
        );
        assert_eq!(
            plan.items[2].status,
            crate::application::bundle::BundleAddStatus::Merge
        );
        assert_eq!(
            plan.items[3].status,
            crate::application::bundle::BundleAddStatus::Conflict
        );
    }

    #[test]
    fn bundle_add_apply_merges_agents_and_preserves_manual_management() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "review": {
                  "source": "./review",
                  "agents": ["pi"]
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");
        let provenance = crate::domain::bundle::BundleProvenance {
            name: crate::domain::bundle::BundleName::new("baseline").unwrap(),
            source: "./bundles/baseline".to_owned(),
        };
        let entries = vec![
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "review".to_owned(),
                original_source: "./review".to_owned(),
                normalized_source: "./review".to_owned(),
            },
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "qa".to_owned(),
                original_source: "./qa".to_owned(),
                normalized_source: "./qa".to_owned(),
            },
        ];
        let plan = store
            .plan_bundle_add(&entries, &["claude".to_owned()], &provenance)
            .unwrap();

        store.apply_bundle_add(&plan).unwrap();

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert_eq!(
            value["dependencies"]["review"]["agents"],
            serde_json::json!(["pi", "claude-code"])
        );
        assert_eq!(
            value["dependencies"]["review"]["managedByBundles"],
            serde_json::Value::Null
        );
        assert_eq!(value["dependencies"]["qa"]["managedByBundles"], true);
    }

    #[test]
    fn bundle_add_plan_does_not_create_missing_config_parent() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("missing-parent/sksync.config.json");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");
        let provenance = crate::domain::bundle::BundleProvenance {
            name: crate::domain::bundle::BundleName::new("baseline").unwrap(),
            source: "./bundles/baseline".to_owned(),
        };
        let entries = vec![crate::application::bundle::LoadedBundleEntry {
            skill_name: "review".to_owned(),
            original_source: "./review".to_owned(),
            normalized_source: "./review".to_owned(),
        }];

        store
            .plan_bundle_add(&entries, &["pi".to_owned()], &provenance)
            .unwrap();

        assert!(!config_path.parent().unwrap().exists());
    }

    #[test]
    fn bundle_remove_plan_classifies_remove_detach_ambiguous_and_not_found() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "bundle-only": {
                  "source": "./one",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }],
                  "managedByBundles": true
                },
                "manual": {
                  "source": "./two",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }]
                },
                "other-source": {
                  "source": "./three",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/b" }]
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");
        let name = crate::domain::bundle::BundleName::new("baseline").unwrap();

        let ambiguous = store.plan_bundle_remove(&name, None).unwrap();
        assert!(ambiguous.is_ambiguous());

        let plan = store
            .plan_bundle_remove(&name, Some("./bundles/a"))
            .unwrap();
        assert_eq!(
            plan.items[0].status,
            crate::application::bundle::BundleRemoveStatus::Remove
        );
        assert_eq!(
            plan.items[1].status,
            crate::application::bundle::BundleRemoveStatus::DetachProvenance
        );

        let missing = store
            .plan_bundle_remove(
                &crate::domain::bundle::BundleName::new("missing").unwrap(),
                None,
            )
            .unwrap();
        assert_eq!(
            missing.items[0].status,
            crate::application::bundle::BundleRemoveStatus::NotFound
        );
    }

    #[test]
    fn bundle_sync_plan_classifies_membership_drift_and_blockers() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "keep-me": {
                  "source": "./keep",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }],
                  "managedByBundles": true
                },
                "source-changed": {
                  "source": "./old",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }],
                  "managedByBundles": true
                },
                "remove-me": {
                  "source": "./removed",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }],
                  "managedByBundles": true
                },
                "detach-me": {
                  "source": "./detached",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }]
                },
                "multi-bundle": {
                  "source": "./multi",
                  "agents": ["pi"],
                  "bundles": [
                    { "name": "baseline", "source": "./bundles/a" },
                    { "name": "other", "source": "./bundles/other" }
                  ],
                  "managedByBundles": true
                },
                "adopt-me": {
                  "source": "./adopt",
                  "agents": ["claude"]
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");
        let provenance = crate::domain::bundle::BundleProvenance {
            name: crate::domain::bundle::BundleName::new("baseline").unwrap(),
            source: "./bundles/a".to_owned(),
        };
        let entries = vec![
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "keep-me".to_owned(),
                original_source: "./keep".to_owned(),
                normalized_source: "./keep".to_owned(),
            },
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "source-changed".to_owned(),
                original_source: "./new".to_owned(),
                normalized_source: "./new".to_owned(),
            },
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "add-me".to_owned(),
                original_source: "./add".to_owned(),
                normalized_source: "./add".to_owned(),
            },
            crate::application::bundle::LoadedBundleEntry {
                skill_name: "adopt-me".to_owned(),
                original_source: "./adopt".to_owned(),
                normalized_source: "./adopt".to_owned(),
            },
        ];

        let plan = store
            .plan_bundle_sync(&provenance, &entries, &[])
            .expect("plan sync");

        assert_eq!(plan.keep_count, 1);
        assert_eq!(
            plan.items
                .iter()
                .map(|item| item.status.as_str())
                .collect::<Vec<_>>(),
            vec![
                "source-changed",
                "add",
                "adopt",
                "detach-provenance",
                "detach-provenance",
                "remove"
            ]
        );
        assert!(plan.has_blockers());
    }

    #[test]
    fn bundle_sync_plan_reports_missing_agents_when_no_agents_can_be_inferred() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "empty-agents": {
                  "source": "./old",
                  "agents": [],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }],
                  "managedByBundles": true
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");
        let provenance = crate::domain::bundle::BundleProvenance {
            name: crate::domain::bundle::BundleName::new("baseline").unwrap(),
            source: "./bundles/a".to_owned(),
        };
        let entries = vec![crate::application::bundle::LoadedBundleEntry {
            skill_name: "new-entry".to_owned(),
            original_source: "./new".to_owned(),
            normalized_source: "./new".to_owned(),
        }];

        let plan = store
            .plan_bundle_sync(&provenance, &entries, &[])
            .expect("plan sync");

        assert_eq!(plan.items[0].status.as_str(), "missing-agents");
        assert!(plan.has_blockers());
    }

    #[test]
    fn bundle_sync_plan_uses_fallback_agents_only_when_inference_is_empty() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "existing": {
                  "source": "./existing",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }],
                  "managedByBundles": true
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");
        let provenance = crate::domain::bundle::BundleProvenance {
            name: crate::domain::bundle::BundleName::new("baseline").unwrap(),
            source: "./bundles/a".to_owned(),
        };
        let entries = vec![crate::application::bundle::LoadedBundleEntry {
            skill_name: "new-entry".to_owned(),
            original_source: "./new".to_owned(),
            normalized_source: "./new".to_owned(),
        }];

        let inferred = store
            .plan_bundle_sync(&provenance, &entries, &["claude".to_owned()])
            .expect("plan sync");

        assert_eq!(inferred.items[0].status.as_str(), "add");
        assert_eq!(inferred.items[0].agents, vec!["pi"]);

        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "existing": {
                  "source": "./existing",
                  "agents": [],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }],
                  "managedByBundles": true
                }
              }
            }"#,
        )
        .expect("rewrite config");

        let fallback = store
            .plan_bundle_sync(&provenance, &entries, &["claude".to_owned()])
            .expect("plan sync");

        assert_eq!(fallback.items[0].status.as_str(), "add");
        assert_eq!(fallback.items[0].agents, vec!["claude-code"]);
    }

    #[test]
    fn bundle_detach_provenance_keeps_manual_dependency() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("sksync.config.json");
        std::fs::write(
            &config_path,
            r#"{
              "dependencies": {
                "review": {
                  "source": "./review",
                  "agents": ["pi"],
                  "bundles": [{ "name": "baseline", "source": "./bundles/a" }]
                }
              }
            }"#,
        )
        .expect("write config");
        let store = FileDependencyConfigStore::new(&config_path, "./.sksync/skills");
        let name = crate::domain::bundle::BundleName::new("baseline").unwrap();
        let plan = store
            .plan_bundle_remove(&name, Some("./bundles/a"))
            .unwrap();

        store.detach_bundle_provenance(&plan).unwrap();

        let value = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&config_path).expect("read config"),
        )
        .expect("parse config");
        assert!(value["dependencies"]["review"].is_object());
        assert_eq!(
            value["dependencies"]["review"]["bundles"],
            serde_json::Value::Null
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
