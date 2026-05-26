use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use thiserror::Error;

use crate::application::source::parse_install_source_string;
use crate::domain::bundle::{
    BundleEntry, BundleManifest, BundleName, BundleNameError, BundleProvenance,
};
use crate::domain::source::{GitInstallSource, InstallSource};
use crate::infrastructure::git::GitClient;
use crate::infrastructure::json::{
    read_bundle_manifest, write_bundle_manifest, BundleExportDependencyConfig,
    BundleManifestJsonError,
};

pub const BUNDLE_MANIFEST_FILE: &str = "sksync.bundle.json";

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
        entries.push(BundleEntry {
            skill_name: skill_name.clone(),
            source: manifest_source.clone(),
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

pub fn load_bundle_from_source(raw_source: &str, config_root: &Path) -> Result<LoadedBundle> {
    let source = parse_install_source_string(raw_source)
        .with_context(|| format!("invalid bundle source {raw_source:?}"))?;
    match source {
        InstallSource::Local(path) => load_local_bundle(raw_source, &path, config_root),
        InstallSource::Git(git) => load_git_bundle(&git),
    }
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
        apply_bundle_export_plan, build_bundle_export_plan, git_source_to_config_string,
        load_bundle_from_source, normalize_bundle_entry_source, validate_snapshot_export_source,
        BundleExportApplyOptions, BundleExportMode, BundleExportPlan, BundleExportPlanInput,
        BundleExportPlanItem, BundleExportResolvedSkill,
    };
    use crate::domain::bundle::{BundleEntry, BundleManifest, BundleName};
    use crate::domain::skill::SkillName;
    use crate::domain::source::GitInstallSource;
    use crate::infrastructure::json::BundleExportDependencyConfig;
    use std::fs;
    use std::path::{Path, PathBuf};

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
            },
            BundleExportDependencyConfig {
                name: "qa".to_owned(),
                source: "./vendor/qa".to_owned(),
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
