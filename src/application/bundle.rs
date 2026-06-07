use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use thiserror::Error;

use crate::application::source::parse_install_source_string;
use crate::domain::bundle::{
    BundleEntry, BundleManifest, BundleName, BundleNameError, BundleProvenance,
};
use crate::domain::package_filter::PackageFilter;
use crate::domain::source::{GitInstallSource, InstallSource};
use crate::infrastructure::git::GitClient;
use crate::infrastructure::json::{
    read_bundle_manifest, write_bundle_manifest, BundleExportDependencyConfig,
    BundleManifestJsonError,
};

pub const BUNDLE_MANIFEST_FILE: &str = "sksync.bundle.json";
pub const BUNDLE_MANIFEST_DISCOVERY_MAX_DEPTH: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleManifestCandidate {
    pub manifest: BundleManifest,
    pub provenance: BundleProvenance,
    pub entries: Vec<LoadedBundleEntry>,
    pub relative_path: PathBuf,
    pub resolved_source: String,
}

impl BundleManifestCandidate {
    pub fn into_loaded_bundle(self) -> LoadedBundle {
        LoadedBundle {
            manifest: self.manifest,
            provenance: self.provenance,
            entries: self.entries,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedBundle {
    pub manifest: BundleManifest,
    pub provenance: BundleProvenance,
    pub entries: Vec<LoadedBundleEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedBundleEntry {
    pub skill_name: String,
    pub original_source: String,
    pub normalized_source: String,
    pub include: Option<PackageFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleAddStatus {
    Create,
    Merge,
    Conflict,
    Skipped,
}

impl BundleAddStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Merge => "merge",
            Self::Conflict => "conflict",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleAddPlanItem {
    pub skill_name: String,
    pub source: String,
    pub include: Option<PackageFilter>,
    pub agents: Vec<String>,
    pub provenance: BundleProvenance,
    pub status: BundleAddStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleAddPlan {
    pub items: Vec<BundleAddPlanItem>,
}

impl BundleAddPlan {
    pub fn has_conflicts(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.status == BundleAddStatus::Conflict)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleRemoveStatus {
    Remove,
    DetachProvenance,
    Ambiguous,
    NotFound,
}

impl BundleRemoveStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remove => "remove",
            Self::DetachProvenance => "detach-provenance",
            Self::Ambiguous => "ambiguous",
            Self::NotFound => "not-found",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleRemovePlanItem {
    pub skill_name: String,
    pub status: BundleRemoveStatus,
    pub source: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleRemovePlan {
    pub bundle: BundleName,
    pub source: Option<String>,
    pub items: Vec<BundleRemovePlanItem>,
    pub ambiguous_sources: Vec<String>,
}

impl BundleRemovePlan {
    pub fn is_ambiguous(&self) -> bool {
        !self.ambiguous_sources.is_empty()
    }

    pub fn is_not_found(&self) -> bool {
        self.items
            .iter()
            .all(|item| item.status == BundleRemoveStatus::NotFound)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleSyncStatus {
    Add,
    Adopt,
    Remove,
    DetachProvenance,
    SourceChanged,
    IncludeChanged,
    MissingAgents,
}

impl BundleSyncStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Adopt => "adopt",
            Self::Remove => "remove",
            Self::DetachProvenance => "detach-provenance",
            Self::SourceChanged => "source-changed",
            Self::IncludeChanged => "include-changed",
            Self::MissingAgents => "missing-agents",
        }
    }

    pub fn is_blocking(self) -> bool {
        matches!(self, Self::SourceChanged | Self::MissingAgents)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleSyncPlanItem {
    pub skill_name: String,
    pub status: BundleSyncStatus,
    pub local_source: Option<String>,
    pub manifest_source: Option<String>,
    pub include: Option<PackageFilter>,
    pub agents: Vec<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleSyncPlan {
    pub bundle: BundleName,
    pub source: String,
    pub items: Vec<BundleSyncPlanItem>,
    pub keep_count: usize,
}

impl BundleSyncPlan {
    pub fn has_blockers(&self) -> bool {
        self.items.iter().any(|item| item.status.is_blocking())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleSyncSourceResolution {
    Resolved(String),
    Ambiguous(Vec<String>),
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleExportMode {
    ManifestOnly,
    Snapshot,
}

impl BundleExportMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManifestOnly => "manifest-only",
            Self::Snapshot => "snapshot",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BundleExportPlanInput {
    pub name: String,
    pub description: Option<String>,
    pub output: PathBuf,
    pub mode: BundleExportMode,
    pub selected_skills: Vec<String>,
    pub dependencies: Vec<BundleExportDependencyConfig>,
    pub resolved_skills: Vec<BundleExportResolvedSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleExportResolvedSkill {
    pub name: String,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleExportPlanItem {
    pub skill_name: String,
    pub manifest_source: String,
    pub source_path: Option<PathBuf>,
    pub snapshot_destination: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleExportPlan {
    pub manifest: BundleManifest,
    pub output: PathBuf,
    pub mode: BundleExportMode,
    pub items: Vec<BundleExportPlanItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleExportApplyOptions {
    pub force: bool,
}

#[derive(Debug, Error)]
pub enum BundleExportError {
    #[error("invalid bundle name '{name}': {source}")]
    InvalidBundleName {
        name: String,
        #[source]
        source: BundleNameError,
    },
    #[error("invalid skill name '{name}': {source}")]
    InvalidSkillName {
        name: String,
        #[source]
        source: crate::domain::skill::SkillNameError,
    },
    #[error("selected skill '{0}' is not a dependency")]
    UnknownSelectedSkill(String),
    #[error("no dependencies selected for export")]
    EmptySelection,
    #[error("invalid dependency source for '{skill}': {message}")]
    InvalidDependencySource { skill: String, message: String },
    #[error("snapshot source for '{skill}' is missing from resolved config")]
    MissingResolvedSkill { skill: String },
    #[error("snapshot source for '{skill}' does not exist: {path}")]
    MissingSnapshotSource { skill: String, path: String },
    #[error("snapshot source for '{skill}' is not a directory: {path}")]
    SnapshotSourceNotDirectory { skill: String, path: String },
    #[error("snapshot source for '{skill}' is invalid SKILL.md: {message}")]
    InvalidSnapshotSkill { skill: String, message: String },
    #[error("bundle export output already exists: {0}")]
    OutputExists(String),
    #[error("failed to create export directory {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to copy snapshot skill {skill} from {from} to {to}: {message}")]
    CopySnapshotSkill {
        skill: String,
        from: String,
        to: String,
        message: String,
    },
    #[error("failed to replace export output {path}: {source}")]
    ReplaceOutput {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write bundle export manifest: {0}")]
    WriteManifest(#[from] BundleManifestJsonError),
}

pub fn build_bundle_export_plan(
    input: BundleExportPlanInput,
) -> std::result::Result<BundleExportPlan, BundleExportError> {
    let bundle_name = BundleName::new(input.name.clone()).map_err(|source| {
        BundleExportError::InvalidBundleName {
            name: input.name.clone(),
            source,
        }
    })?;
    let selected = input
        .selected_skills
        .iter()
        .map(|skill| skill.trim().to_owned())
        .filter(|skill| !skill.is_empty())
        .collect::<BTreeSet<_>>();
    let dependencies = input
        .dependencies
        .into_iter()
        .map(|dependency| (dependency.name.clone(), dependency))
        .collect::<BTreeMap<_, _>>();

    for selected_skill in &selected {
        if !dependencies.contains_key(selected_skill) {
            return Err(BundleExportError::UnknownSelectedSkill(
                selected_skill.clone(),
            ));
        }
    }

    let resolved = input
        .resolved_skills
        .into_iter()
        .map(|skill| (skill.name.clone(), skill.source_path))
        .collect::<BTreeMap<_, _>>();

    let mut items = Vec::new();
    let mut entries = Vec::new();
    for (name, dependency) in dependencies {
        if !selected.is_empty() && !selected.contains(&name) {
            continue;
        }
        let skill_name = crate::domain::skill::SkillName::new(name.clone()).map_err(|source| {
            BundleExportError::InvalidSkillName {
                name: name.clone(),
                source,
            }
        })?;
        parse_install_source_string(&dependency.source).map_err(|error| {
            BundleExportError::InvalidDependencySource {
                skill: name.clone(),
                message: error.to_string(),
            }
        })?;
        let manifest_source = match input.mode {
            BundleExportMode::ManifestOnly => dependency.source.clone(),
            BundleExportMode::Snapshot => format!("./skills/{name}"),
        };
        let source_path = match input.mode {
            BundleExportMode::ManifestOnly => None,
            BundleExportMode::Snapshot => Some(resolved.get(&name).cloned().ok_or_else(|| {
                BundleExportError::MissingResolvedSkill {
                    skill: name.clone(),
                }
            })?),
        };
        let snapshot_destination = source_path
            .as_ref()
            .map(|_| input.output.join("skills").join(&name));
        let include = match input.mode {
            BundleExportMode::ManifestOnly => dependency.include.clone(),
            BundleExportMode::Snapshot => None,
        };
        entries.push(BundleEntry {
            skill_name: skill_name.clone(),
            source: manifest_source.clone(),
            include,
        });
        items.push(BundleExportPlanItem {
            skill_name: skill_name.as_str().to_owned(),
            manifest_source,
            source_path,
            snapshot_destination,
        });
    }

    if items.is_empty() {
        return Err(BundleExportError::EmptySelection);
    }

    Ok(BundleExportPlan {
        manifest: BundleManifest {
            name: bundle_name,
            description: input
                .description
                .unwrap_or_else(|| "Exported from sksync config.".to_owned()),
            entries,
        },
        output: input.output,
        mode: input.mode,
        items,
    })
}

pub fn validate_snapshot_export_source(
    skill: &str,
    path: &Path,
) -> std::result::Result<(), BundleExportError> {
    if !path.exists() {
        return Err(BundleExportError::MissingSnapshotSource {
            skill: skill.to_owned(),
            path: path.display().to_string(),
        });
    }
    if !path.is_dir() {
        return Err(BundleExportError::SnapshotSourceNotDirectory {
            skill: skill.to_owned(),
            path: path.display().to_string(),
        });
    }
    let skill_md = path.join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md).map_err(|error| {
        BundleExportError::InvalidSnapshotSkill {
            skill: skill.to_owned(),
            message: error.to_string(),
        }
    })?;
    crate::domain::skill_manifest::parse_skill_manifest(&content).map_err(|error| {
        BundleExportError::InvalidSnapshotSkill {
            skill: skill.to_owned(),
            message: error.to_string(),
        }
    })?;
    Ok(())
}

pub fn validate_bundle_export_plan(
    plan: &BundleExportPlan,
) -> std::result::Result<(), BundleExportError> {
    if plan.mode == BundleExportMode::Snapshot {
        for item in &plan.items {
            let source = item.source_path.as_ref().ok_or_else(|| {
                BundleExportError::MissingResolvedSkill {
                    skill: item.skill_name.clone(),
                }
            })?;
            validate_snapshot_export_source(&item.skill_name, source)?;
        }
    }
    Ok(())
}

pub fn apply_bundle_export_plan(
    plan: &BundleExportPlan,
    options: BundleExportApplyOptions,
) -> std::result::Result<(), BundleExportError> {
    if plan.output.exists() && !options.force {
        return Err(BundleExportError::OutputExists(
            plan.output.display().to_string(),
        ));
    }
    let parent = plan.output.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|source| BundleExportError::CreateDir {
        path: parent.display().to_string(),
        source,
    })?;
    let staging = temporary_bundle_export_staging_dir(&plan.output);
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|source| BundleExportError::ReplaceOutput {
            path: staging.display().to_string(),
            source,
        })?;
    }
    std::fs::create_dir_all(&staging).map_err(|source| BundleExportError::CreateDir {
        path: staging.display().to_string(),
        source,
    })?;

    let result = (|| -> std::result::Result<(), BundleExportError> {
        validate_bundle_export_plan(plan)?;
        if plan.mode == BundleExportMode::Snapshot {
            for item in &plan.items {
                let source = item.source_path.as_ref().ok_or_else(|| {
                    BundleExportError::MissingResolvedSkill {
                        skill: item.skill_name.clone(),
                    }
                })?;
                let destination = staging.join("skills").join(&item.skill_name);
                copy_dir_all_for_bundle_export(source, &destination, &item.skill_name)?;
            }
        }
        write_bundle_manifest(staging.join(BUNDLE_MANIFEST_FILE), &plan.manifest)?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }

    if let Err(error) = replace_bundle_export_output(&staging, &plan.output) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    Ok(())
}

fn replace_bundle_export_output(
    staging: &Path,
    output: &Path,
) -> std::result::Result<(), BundleExportError> {
    if output.exists() {
        if output.is_dir() {
            std::fs::remove_dir_all(output).map_err(|source| BundleExportError::ReplaceOutput {
                path: output.display().to_string(),
                source,
            })?;
        } else {
            std::fs::remove_file(output).map_err(|source| BundleExportError::ReplaceOutput {
                path: output.display().to_string(),
                source,
            })?;
        }
    }
    std::fs::rename(staging, output).map_err(|source| BundleExportError::ReplaceOutput {
        path: output.display().to_string(),
        source,
    })
}

fn copy_dir_all_for_bundle_export(
    from: &Path,
    to: &Path,
    skill: &str,
) -> std::result::Result<(), BundleExportError> {
    std::fs::create_dir_all(to).map_err(|source| BundleExportError::CreateDir {
        path: to.display().to_string(),
        source,
    })?;
    for entry in std::fs::read_dir(from).map_err(|source| BundleExportError::CopySnapshotSkill {
        skill: skill.to_owned(),
        from: from.display().to_string(),
        to: to.display().to_string(),
        message: source.to_string(),
    })? {
        let entry = entry.map_err(|source| BundleExportError::CopySnapshotSkill {
            skill: skill.to_owned(),
            from: from.display().to_string(),
            to: to.display().to_string(),
            message: source.to_string(),
        })?;
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        let file_type =
            entry
                .file_type()
                .map_err(|source| BundleExportError::CopySnapshotSkill {
                    skill: skill.to_owned(),
                    from: source_path.display().to_string(),
                    to: target_path.display().to_string(),
                    message: source.to_string(),
                })?;
        if file_type.is_dir() {
            copy_dir_all_for_bundle_export(&source_path, &target_path, skill)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &target_path).map_err(|source| {
                BundleExportError::CopySnapshotSkill {
                    skill: skill.to_owned(),
                    from: source_path.display().to_string(),
                    to: target_path.display().to_string(),
                    message: source.to_string(),
                }
            })?;
        }
    }
    Ok(())
}

fn temporary_bundle_export_staging_dir(output: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bundle-export");
    output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{file_name}.sksync-export-staging-{}-{nonce}",
            std::process::id()
        ))
}

pub fn discover_bundle_manifest_candidates(
    raw_source: &str,
    config_root: &Path,
) -> Result<Vec<BundleManifestCandidate>> {
    let source = parse_install_source_string(raw_source)
        .with_context(|| format!("invalid bundle source {raw_source:?}"))?;
    match source {
        InstallSource::Local(path) => discover_local_bundle_manifest_candidates(&path, config_root),
        InstallSource::Git(git) => discover_git_bundle_manifest_candidates(&git),
    }
}

pub fn load_bundle_from_source(raw_source: &str, config_root: &Path) -> Result<LoadedBundle> {
    let source = parse_install_source_string(raw_source)
        .with_context(|| format!("invalid bundle source {raw_source:?}"))?;
    match source {
        InstallSource::Local(path) => load_local_bundle(raw_source, &path, config_root),
        InstallSource::Git(git) => load_git_bundle(&git),
    }
}

fn discover_local_bundle_manifest_candidates(
    path: &Path,
    config_root: &Path,
) -> Result<Vec<BundleManifestCandidate>> {
    let resolved = absolutize_config_path(path, config_root);
    let root = if is_bundle_manifest_file_path(&resolved) {
        resolved
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        resolved
    };

    if root.join(BUNDLE_MANIFEST_FILE).is_file() {
        return Ok(vec![load_local_bundle_manifest_candidate(
            &root,
            &root,
            config_root,
        )?]);
    }

    let mut candidates = Vec::new();
    discover_local_bundle_manifest_candidates_inner(
        &root,
        &root,
        config_root,
        BUNDLE_MANIFEST_DISCOVERY_MAX_DEPTH,
        0,
        &mut candidates,
    )?;
    candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(candidates)
}

fn discover_local_bundle_manifest_candidates_inner(
    root: &Path,
    current: &Path,
    config_root: &Path,
    max_depth: usize,
    depth: usize,
    candidates: &mut Vec<BundleManifestCandidate>,
) -> Result<()> {
    if depth > max_depth || is_skipped_bundle_discovery_dir(current) {
        return Ok(());
    }

    if current.join(BUNDLE_MANIFEST_FILE).is_file() {
        candidates.push(load_local_bundle_manifest_candidate(
            root,
            current,
            config_root,
        )?);
        return Ok(());
    }

    for entry in std::fs::read_dir(current)
        .with_context(|| format!("failed to read directory {}", current.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", current.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        if file_type.is_dir() {
            discover_local_bundle_manifest_candidates_inner(
                root,
                &entry.path(),
                config_root,
                max_depth,
                depth + 1,
                candidates,
            )?;
        }
    }

    Ok(())
}

fn load_local_bundle_manifest_candidate(
    root: &Path,
    manifest_dir: &Path,
    config_root: &Path,
) -> Result<BundleManifestCandidate> {
    let manifest = read_bundle_manifest(manifest_dir.join(BUNDLE_MANIFEST_FILE))?;
    let resolved_source = normalize_local_source_for_config(manifest_dir, config_root);
    let entries = manifest
        .entries
        .iter()
        .map(|entry| {
            let normalized_source = normalize_bundle_entry_source(
                &entry.source,
                Some(manifest_dir),
                None,
                config_root,
            )?;
            Ok(LoadedBundleEntry {
                skill_name: entry.skill_name.as_str().to_owned(),
                original_source: entry.source.clone(),
                normalized_source,
                include: entry.include.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(BundleManifestCandidate {
        provenance: BundleProvenance {
            name: manifest.name.clone(),
            source: resolved_source.clone(),
        },
        manifest,
        entries,
        relative_path: relative_bundle_manifest_path(root, manifest_dir),
        resolved_source,
    })
}

fn discover_git_bundle_manifest_candidates(
    git: &GitInstallSource,
) -> Result<Vec<BundleManifestCandidate>> {
    let base_git = bundle_manifest_parent_git_source(git)?;
    let clone_dir = temporary_bundle_clone_dir();
    let result = (|| {
        GitClient.clone_checkout(&base_git, &clone_dir)?;
        let search_root = clone_dir.join(&base_git.path);
        if search_root.join(BUNDLE_MANIFEST_FILE).is_file() {
            return Ok(vec![load_git_bundle_manifest_candidate(
                &clone_dir,
                &search_root,
                &search_root,
                &base_git,
            )?]);
        }

        let mut candidates = Vec::new();
        discover_git_bundle_manifest_candidates_inner(
            &clone_dir,
            &search_root,
            &search_root,
            &base_git,
            BUNDLE_MANIFEST_DISCOVERY_MAX_DEPTH,
            0,
            &mut candidates,
        )?;
        candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(candidates)
    })();

    if clone_dir.exists() {
        let _ = std::fs::remove_dir_all(&clone_dir);
    }

    result
}

fn discover_git_bundle_manifest_candidates_inner(
    clone_root: &Path,
    search_root: &Path,
    current: &Path,
    base_git: &GitInstallSource,
    max_depth: usize,
    depth: usize,
    candidates: &mut Vec<BundleManifestCandidate>,
) -> Result<()> {
    if depth > max_depth || is_skipped_bundle_discovery_dir(current) {
        return Ok(());
    }

    if current.join(BUNDLE_MANIFEST_FILE).is_file() {
        candidates.push(load_git_bundle_manifest_candidate(
            clone_root,
            search_root,
            current,
            base_git,
        )?);
        return Ok(());
    }

    for entry in std::fs::read_dir(current)
        .with_context(|| format!("failed to read directory {}", current.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", current.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        if file_type.is_dir() {
            discover_git_bundle_manifest_candidates_inner(
                clone_root,
                search_root,
                &entry.path(),
                base_git,
                max_depth,
                depth + 1,
                candidates,
            )?;
        }
    }

    Ok(())
}

fn load_git_bundle_manifest_candidate(
    clone_root: &Path,
    search_root: &Path,
    manifest_dir: &Path,
    base_git: &GitInstallSource,
) -> Result<BundleManifestCandidate> {
    let manifest = read_bundle_manifest(manifest_dir.join(BUNDLE_MANIFEST_FILE))?;
    let selected_git_path = manifest_dir
        .strip_prefix(clone_root)
        .unwrap_or(manifest_dir)
        .to_path_buf();
    let selected_git_path = if selected_git_path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        selected_git_path
    };
    let resolved_git = GitInstallSource {
        url: base_git.url.clone(),
        reference: base_git.reference.clone(),
        path: selected_git_path,
    };
    let resolved_source = git_source_to_config_string(&resolved_git);
    let entries = manifest
        .entries
        .iter()
        .map(|entry| {
            let normalized_source = normalize_bundle_entry_source(
                &entry.source,
                None,
                Some(&resolved_git),
                Path::new("."),
            )?;
            Ok(LoadedBundleEntry {
                skill_name: entry.skill_name.as_str().to_owned(),
                original_source: entry.source.clone(),
                normalized_source,
                include: entry.include.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(BundleManifestCandidate {
        provenance: BundleProvenance {
            name: manifest.name.clone(),
            source: resolved_source.clone(),
        },
        manifest,
        entries,
        relative_path: relative_bundle_manifest_path(search_root, manifest_dir),
        resolved_source,
    })
}

fn bundle_manifest_parent_git_source(git: &GitInstallSource) -> Result<GitInstallSource> {
    if !is_bundle_manifest_file_path(&git.path) {
        return Ok(git.clone());
    }
    let parent = git
        .path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(GitInstallSource {
        url: git.url.clone(),
        reference: git.reference.clone(),
        path: parent,
    })
}

fn relative_bundle_manifest_path(root: &Path, manifest_dir: &Path) -> PathBuf {
    let relative_path = manifest_dir
        .strip_prefix(root)
        .unwrap_or(manifest_dir)
        .to_path_buf();
    if relative_path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        relative_path
    }
}

fn is_skipped_bundle_discovery_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "node_modules" | ".sksync"))
}

fn is_bundle_manifest_file_path(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some(BUNDLE_MANIFEST_FILE)
}

fn load_local_bundle(raw_source: &str, path: &Path, config_root: &Path) -> Result<LoadedBundle> {
    let manifest_dir = absolutize_config_path(path, config_root);
    let manifest = read_bundle_manifest(manifest_dir.join(BUNDLE_MANIFEST_FILE))?;
    let provenance_source = normalize_local_source_for_config(&manifest_dir, config_root);
    let entries = manifest
        .entries
        .iter()
        .map(|entry| {
            let normalized_source = normalize_bundle_entry_source(
                &entry.source,
                Some(&manifest_dir),
                None,
                config_root,
            )?;
            Ok(LoadedBundleEntry {
                skill_name: entry.skill_name.as_str().to_owned(),
                original_source: entry.source.clone(),
                normalized_source,
                include: entry.include.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let provenance = BundleProvenance {
        name: manifest.name.clone(),
        source: if raw_source.starts_with("./") || raw_source.starts_with("../") {
            provenance_source
        } else {
            normalize_local_source_for_config(&manifest_dir, config_root)
        },
    };

    Ok(LoadedBundle {
        manifest,
        provenance,
        entries,
    })
}

fn load_git_bundle(git: &GitInstallSource) -> Result<LoadedBundle> {
    let clone_dir = temporary_bundle_clone_dir();
    let result = (|| {
        GitClient.clone_checkout(git, &clone_dir)?;
        let manifest_dir = clone_dir.join(&git.path);
        let manifest = read_bundle_manifest(manifest_dir.join(BUNDLE_MANIFEST_FILE))?;
        let provenance = BundleProvenance {
            name: manifest.name.clone(),
            source: git_source_to_config_string(git),
        };
        let entries = manifest
            .entries
            .iter()
            .map(|entry| {
                let normalized_source =
                    normalize_bundle_entry_source(&entry.source, None, Some(git), Path::new("."))?;
                Ok(LoadedBundleEntry {
                    skill_name: entry.skill_name.as_str().to_owned(),
                    original_source: entry.source.clone(),
                    normalized_source,
                    include: entry.include.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(LoadedBundle {
            manifest,
            provenance,
            entries,
        })
    })();

    if clone_dir.exists() {
        let _ = std::fs::remove_dir_all(&clone_dir);
    }

    result
}

fn normalize_bundle_entry_source(
    source: &str,
    local_manifest_dir: Option<&Path>,
    git_manifest_source: Option<&GitInstallSource>,
    config_root: &Path,
) -> Result<String> {
    if is_relative_local_source(source) {
        if let Some(manifest_dir) = local_manifest_dir {
            let resolved = manifest_dir.join(source);
            return Ok(normalize_local_source_for_config(&resolved, config_root));
        }
        if let Some(git) = git_manifest_source {
            let relative = source.trim_start_matches("./");
            let path = normalize_git_join(&git.path, Path::new(relative))?;
            return Ok(git_source_to_config_string(&GitInstallSource {
                url: git.url.clone(),
                reference: git.reference.clone(),
                path,
            }));
        }
    }

    let parsed = parse_install_source_string(source)
        .with_context(|| format!("invalid bundle entry source {source:?}"))?;
    Ok(match parsed {
        InstallSource::Local(path) => normalize_local_source_for_config(
            &absolutize_config_path(&path, config_root),
            config_root,
        ),
        InstallSource::Git(git) => git_source_to_config_string(&git),
    })
}

fn is_relative_local_source(source: &str) -> bool {
    source.starts_with("./") || source.starts_with("../")
}

fn absolutize_config_path(path: &Path, config_root: &Path) -> PathBuf {
    if path.is_absolute() || is_tilde_path(path) {
        path.to_path_buf()
    } else {
        config_root.join(path)
    }
}

fn is_tilde_path(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|value| value == "~" || value.starts_with("~/"))
}

fn normalize_local_source_for_config(path: &Path, config_root: &Path) -> String {
    let normalized = normalize_path_without_fs(path);
    let root = normalize_path_without_fs(config_root);
    if let Ok(relative) = normalized.strip_prefix(&root) {
        let value = relative.to_string_lossy().replace('\\', "/");
        if value.is_empty() {
            ".".to_owned()
        } else {
            format!("./{value}")
        }
    } else {
        normalized.to_string_lossy().replace('\\', "/")
    }
}

fn normalize_path_without_fs(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn normalize_git_join(base: &Path, relative: &Path) -> Result<PathBuf> {
    let mut joined = if base == Path::new(".") {
        PathBuf::new()
    } else {
        base.to_path_buf()
    };
    joined.push(relative);
    let normalized = normalize_path_without_fs(&joined);
    if normalized
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!(
            "bundle entry source escapes git source path: {}",
            relative.display()
        );
    }
    if normalized.as_os_str().is_empty() {
        Ok(PathBuf::from("."))
    } else {
        Ok(normalized)
    }
}

pub fn git_source_to_config_string(git: &GitInstallSource) -> String {
    if let Some(repo) = github_repo_from_url(&git.url) {
        let reference = git.reference.as_deref().unwrap_or("HEAD");
        if git.path == Path::new(".") {
            format!("https://github.com/{repo}/tree/{reference}")
        } else {
            format!(
                "https://github.com/{repo}/tree/{}/{}",
                reference,
                git.path.to_string_lossy().replace('\\', "/")
            )
        }
    } else if git.path == Path::new(".") {
        git.url.clone()
    } else if let Some(reference) = &git.reference {
        format!("{}#{}", git.url, reference)
    } else {
        git.url.clone()
    }
}

fn github_repo_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("git@github.com:"))?;
    let parts = rest
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() >= 2 {
        Some(format!("{}/{}", parts[0], parts[1]))
    } else {
        None
    }
}

fn temporary_bundle_clone_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("sksync-bundle-{}-{nonce}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::{
        apply_bundle_export_plan, build_bundle_export_plan, bundle_manifest_parent_git_source,
        discover_bundle_manifest_candidates, git_source_to_config_string, load_bundle_from_source,
        normalize_bundle_entry_source, validate_snapshot_export_source, BundleExportApplyOptions,
        BundleExportMode, BundleExportPlan, BundleExportPlanInput, BundleExportPlanItem,
        BundleExportResolvedSkill,
    };
    use crate::domain::bundle::{BundleEntry, BundleManifest, BundleName};
    use crate::domain::skill::SkillName;
    use crate::domain::source::GitInstallSource;
    use crate::infrastructure::json::BundleExportDependencyConfig;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn write_bundle_manifest(path: &Path, name: &str, description: &str) {
        fs::create_dir_all(path).unwrap();
        fs::write(
            path.join("sksync.bundle.json"),
            format!(
                r#"{{
                  "name": "{name}",
                  "description": "{description}",
                  "entries": {{ "review": {{ "source": "./skills/review" }} }}
                }}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn discovers_bundle_manifests_under_local_source() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("repo");
        write_bundle_manifest(&root.join("bundles/base"), "team-baseline", "Team baseline");
        write_bundle_manifest(
            &root.join("node_modules/ignored"),
            "ignored",
            "Ignored bundle",
        );

        let candidates = discover_bundle_manifest_candidates("./repo", temp.path())
            .expect("discover candidates");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].manifest.name.as_str(), "team-baseline");
        assert_eq!(candidates[0].relative_path, Path::new("bundles/base"));
        assert_eq!(candidates[0].resolved_source, "./repo/bundles/base");
        assert_eq!(candidates[0].provenance.source, "./repo/bundles/base");
        assert_eq!(
            candidates[0].entries[0].normalized_source,
            "./repo/bundles/base/skills/review"
        );
    }

    #[test]
    fn direct_bundle_manifest_file_resolves_to_parent_source() {
        let temp = tempfile::tempdir().expect("temp dir");
        let bundle_dir = temp.path().join("repo/bundles/base");
        write_bundle_manifest(&bundle_dir, "team-baseline", "Team baseline");

        let candidates = discover_bundle_manifest_candidates(
            "./repo/bundles/base/sksync.bundle.json",
            temp.path(),
        )
        .expect("discover candidates");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].relative_path, Path::new("."));
        assert_eq!(candidates[0].resolved_source, "./repo/bundles/base");
    }

    #[test]
    fn git_bundle_manifest_file_source_resolves_to_parent_tree_source() {
        let git = GitInstallSource {
            url: "https://github.com/org/bundles.git".to_owned(),
            reference: Some("main".to_owned()),
            path: PathBuf::from("bundles/base/sksync.bundle.json"),
        };

        let parent = bundle_manifest_parent_git_source(&git).unwrap();

        assert_eq!(parent.path, Path::new("bundles/base"));
        assert_eq!(
            git_source_to_config_string(&parent),
            "https://github.com/org/bundles/tree/main/bundles/base"
        );
    }

    #[test]
    fn local_bundle_relative_entries_are_config_root_relative() {
        let temp = std::env::temp_dir().join(format!("sksync-bundle-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        let bundle_dir = temp.join("bundles/review");
        fs::create_dir_all(bundle_dir.join("skills/review")).unwrap();
        fs::write(
            bundle_dir.join("sksync.bundle.json"),
            r#"{
              "name": "review-workflow",
              "description": "Review workflow skills.",
              "entries": { "review": { "source": "./skills/review" } }
            }"#,
        )
        .unwrap();

        let loaded = load_bundle_from_source("./bundles/review", &temp).unwrap();

        assert_eq!(loaded.provenance.source, "./bundles/review");
        assert_eq!(
            loaded.entries[0].normalized_source,
            "./bundles/review/skills/review"
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn git_relative_entries_use_same_repo_and_ref() {
        let git = GitInstallSource {
            url: "https://github.com/org/bundles.git".to_owned(),
            reference: Some("main".to_owned()),
            path: PathBuf::from("bundles/review"),
        };

        let source =
            normalize_bundle_entry_source("./skills/review", None, Some(&git), Path::new("."))
                .unwrap();

        assert_eq!(
            source,
            "https://github.com/org/bundles/tree/main/bundles/review/skills/review"
        );
    }

    #[test]
    fn github_sources_normalize_to_tree_urls() {
        let source = git_source_to_config_string(&GitInstallSource {
            url: "https://github.com/org/repo.git".to_owned(),
            reference: Some("v1".to_owned()),
            path: PathBuf::from("skills/review"),
        });

        assert_eq!(source, "https://github.com/org/repo/tree/v1/skills/review");
    }

    #[test]
    fn manifest_only_export_plan_preserves_dependency_sources() {
        let dependencies = vec![
            BundleExportDependencyConfig {
                name: "review".to_owned(),
                source: "github:org/repo/skills/review#main".to_owned(),
                include: Some(crate::domain::package_filter::PackageFilter::manifest_only()),
            },
            BundleExportDependencyConfig {
                name: "qa".to_owned(),
                source: "./vendor/qa".to_owned(),
                include: None,
            },
        ];
        let plan = build_bundle_export_plan(BundleExportPlanInput {
            name: "team-baseline".to_owned(),
            description: None,
            output: PathBuf::from("./bundles/team-baseline"),
            mode: BundleExportMode::ManifestOnly,
            selected_skills: Vec::new(),
            dependencies,
            resolved_skills: Vec::new(),
        })
        .unwrap();

        assert_eq!(plan.manifest.name.as_str(), "team-baseline");
        assert_eq!(plan.items[0].skill_name, "qa");
        assert_eq!(plan.items[0].manifest_source, "./vendor/qa");
        assert_eq!(plan.items[1].skill_name, "review");
        assert_eq!(
            plan.items[1].manifest_source,
            "github:org/repo/skills/review#main"
        );
        assert_eq!(
            plan.manifest.entries[1].include,
            Some(crate::domain::package_filter::PackageFilter::manifest_only())
        );
    }

    #[test]
    fn export_plan_rejects_selected_skill_not_in_dependencies() {
        let error = build_bundle_export_plan(BundleExportPlanInput {
            name: "team-baseline".to_owned(),
            description: None,
            output: PathBuf::from("./bundles/team-baseline"),
            mode: BundleExportMode::ManifestOnly,
            selected_skills: vec!["missing".to_owned()],
            dependencies: Vec::new(),
            resolved_skills: Vec::new(),
        })
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("selected skill 'missing' is not a dependency"));
    }

    #[test]
    fn snapshot_export_plan_rewrites_sources_to_manifest_relative_paths() {
        let dependencies = vec![BundleExportDependencyConfig {
            name: "review".to_owned(),
            source: "github:org/repo/skills/review#main".to_owned(),
            include: Some(crate::domain::package_filter::PackageFilter::manifest_only()),
        }];
        let resolved_skills = vec![BundleExportResolvedSkill {
            name: "review".to_owned(),
            source_path: PathBuf::from("./.sksync/skills/org/repo/review"),
        }];

        let plan = build_bundle_export_plan(BundleExportPlanInput {
            name: "team-baseline".to_owned(),
            description: None,
            output: PathBuf::from("./bundles/team-baseline"),
            mode: BundleExportMode::Snapshot,
            selected_skills: Vec::new(),
            dependencies,
            resolved_skills,
        })
        .unwrap();

        assert_eq!(plan.items[0].manifest_source, "./skills/review");
        assert_eq!(
            plan.items[0].source_path.as_deref(),
            Some(Path::new("./.sksync/skills/org/repo/review"))
        );
        assert_eq!(
            plan.items[0].snapshot_destination.as_deref(),
            Some(Path::new("./bundles/team-baseline/skills/review"))
        );
        assert_eq!(plan.manifest.entries[0].include, None);
    }

    #[test]
    fn validate_snapshot_export_source_requires_valid_skill_manifest() {
        let temp = tempfile::tempdir().expect("temp dir");
        let skill_dir = temp.path().join("review");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Missing frontmatter\n").unwrap();

        let error = validate_snapshot_export_source("review", &skill_dir).unwrap_err();

        assert!(error.to_string().contains("invalid SKILL.md"));
    }

    #[test]
    fn validate_snapshot_export_source_accepts_valid_skill_manifest() {
        let temp = tempfile::tempdir().expect("temp dir");
        let skill_dir = temp.path().join("review");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: review\ndescription: Review skill\n---\n# Review\n",
        )
        .unwrap();

        validate_snapshot_export_source("review", &skill_dir).unwrap();
    }

    #[test]
    fn apply_manifest_only_export_refuses_existing_output_without_force() {
        let temp = tempfile::tempdir().expect("temp dir");
        let output = temp.path().join("bundle");
        fs::create_dir_all(&output).unwrap();
        let plan = export_plan_for_test(output.clone(), BundleExportMode::ManifestOnly);

        let error =
            apply_bundle_export_plan(&plan, BundleExportApplyOptions { force: false }).unwrap_err();

        assert!(error.to_string().contains("already exists"));
    }

    #[test]
    fn apply_manifest_only_export_writes_manifest_without_skills_dir() {
        let temp = tempfile::tempdir().expect("temp dir");
        let output = temp.path().join("bundle");
        let plan = export_plan_for_test(output.clone(), BundleExportMode::ManifestOnly);

        apply_bundle_export_plan(&plan, BundleExportApplyOptions { force: false }).unwrap();

        assert!(output.join("sksync.bundle.json").is_file());
        assert!(!output.join("skills").exists());
    }

    #[test]
    fn apply_snapshot_export_copies_skills_and_manifest() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("installed/review");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: review\ndescription: Review skill\n---\n# Review\n",
        )
        .unwrap();
        let output = temp.path().join("bundle");
        let mut plan = export_plan_for_test(output.clone(), BundleExportMode::Snapshot);
        plan.items[0].source_path = Some(source.clone());
        plan.items[0].snapshot_destination = Some(output.join("skills/review"));

        apply_bundle_export_plan(&plan, BundleExportApplyOptions { force: false }).unwrap();

        assert!(output.join("sksync.bundle.json").is_file());
        assert!(output.join("skills/review/SKILL.md").is_file());
    }

    fn export_plan_for_test(output: PathBuf, mode: BundleExportMode) -> BundleExportPlan {
        let manifest_source = match mode {
            BundleExportMode::ManifestOnly => "github:org/repo/skills/review#main".to_owned(),
            BundleExportMode::Snapshot => "./skills/review".to_owned(),
        };
        BundleExportPlan {
            manifest: BundleManifest {
                name: BundleName::new("team-baseline").unwrap(),
                description: "Exported from sksync config.".to_owned(),
                entries: vec![BundleEntry {
                    skill_name: SkillName::new("review").unwrap(),
                    source: manifest_source.clone(),
                    include: None,
                }],
            },
            output: output.clone(),
            mode,
            items: vec![BundleExportPlanItem {
                skill_name: "review".to_owned(),
                manifest_source,
                source_path: None,
                snapshot_destination: if mode == BundleExportMode::Snapshot {
                    Some(output.join("skills/review"))
                } else {
                    None
                },
            }],
        }
    }
}
