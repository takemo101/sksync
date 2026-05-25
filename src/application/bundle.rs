use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::application::source::parse_install_source_string;
use crate::domain::bundle::{BundleManifest, BundleName, BundleProvenance};
use crate::domain::source::{GitInstallSource, InstallSource};
use crate::infrastructure::git::GitClient;
use crate::infrastructure::json::read_bundle_manifest;

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
        git_source_to_config_string, load_bundle_from_source, normalize_bundle_entry_source,
    };
    use crate::domain::source::GitInstallSource;
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
}
