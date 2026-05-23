use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::domain::skill_manifest::parse_skill_manifest;
use crate::domain::source::{GitInstallSource, InstallSource};
use crate::infrastructure::git::GitClient;

#[derive(Debug, Clone)]
pub struct SkillCandidate {
    pub name: String,
    pub description: String,
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRewriteMode {
    Append,
    ReplaceSkillsShPath,
}

#[derive(Debug, Clone)]
pub struct DiscoveredSkills {
    pub candidates: Vec<SkillCandidate>,
    pub rewrite_mode: SourceRewriteMode,
    pub default_selection_name: Option<String>,
}

pub fn discover_source_skills(
    source: &InstallSource,
    raw_source: &str,
) -> Result<DiscoveredSkills> {
    match source {
        InstallSource::Local(path) => Ok(DiscoveredSkills {
            candidates: discover_skill_candidates(path, 5)?,
            rewrite_mode: SourceRewriteMode::Append,
            default_selection_name: None,
        }),
        InstallSource::Git(git) => discover_git_source_skills(git, raw_source),
    }
}

fn discover_git_source_skills(
    source: &GitInstallSource,
    raw_source: &str,
) -> Result<DiscoveredSkills> {
    let clone_dir = temporary_clone_dir();
    let result = (|| {
        GitClient.clone_checkout(source, &clone_dir)?;
        let search_dir = clone_dir.join(&source.path);
        if search_dir.exists() {
            return Ok(DiscoveredSkills {
                candidates: discover_skill_candidates(&search_dir, 5)?,
                rewrite_mode: SourceRewriteMode::Append,
                default_selection_name: None,
            });
        }

        if is_skills_sh_source_body(split_source_reference(raw_source).0)
            && source.path != Path::new(".")
        {
            return Ok(DiscoveredSkills {
                candidates: discover_skill_candidates(&clone_dir, 5)?,
                rewrite_mode: SourceRewriteMode::ReplaceSkillsShPath,
                default_selection_name: Some(infer_skill_name(raw_source)),
            });
        }

        bail!(
            "install source path does not exist: {}",
            search_dir.display()
        );
    })();

    if clone_dir.exists() {
        let _ = fs::remove_dir_all(&clone_dir);
    }

    result
}

fn temporary_clone_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("sksync-discover-{}-{nonce}", std::process::id()))
}

pub fn discover_skill_candidates(root: &Path, max_depth: usize) -> Result<Vec<SkillCandidate>> {
    let mut candidates = Vec::new();
    discover_skill_candidates_inner(root, root, max_depth, 0, &mut candidates)?;
    candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(candidates)
}

fn discover_skill_candidates_inner(
    root: &Path,
    current: &Path,
    max_depth: usize,
    depth: usize,
    candidates: &mut Vec<SkillCandidate>,
) -> Result<()> {
    if depth > max_depth || is_skipped_discovery_dir(current) {
        return Ok(());
    }

    let skill_md = current.join("SKILL.md");
    if skill_md.is_file() {
        let metadata = read_skill_metadata(current)?;
        let relative_path = current.strip_prefix(root).unwrap_or(current).to_path_buf();
        candidates.push(SkillCandidate {
            name: metadata.0,
            description: metadata.1,
            relative_path: if relative_path.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative_path
            },
        });
        return Ok(());
    }

    for entry in fs::read_dir(current)
        .with_context(|| format!("failed to read directory {}", current.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", current.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        if file_type.is_dir() {
            discover_skill_candidates_inner(root, &entry.path(), max_depth, depth + 1, candidates)?;
        }
    }

    Ok(())
}

fn is_skipped_discovery_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "node_modules" | ".sksync"))
}

fn read_skill_metadata(path: &Path) -> Result<(String, String)> {
    let skill_md = path.join("SKILL.md");
    let content = fs::read_to_string(&skill_md)
        .with_context(|| format!("failed to read {}", skill_md.display()))?;
    let manifest = parse_skill_manifest(&content)
        .with_context(|| format!("invalid SKILL.md metadata: {}", skill_md.display()))?;
    Ok((manifest.name, manifest.description))
}

pub fn source_with_selected_subpath(
    source: &str,
    relative_path: &Path,
    rewrite_mode: SourceRewriteMode,
) -> String {
    if relative_path == Path::new(".") {
        return source.to_owned();
    }

    let (body, reference) = split_source_reference(source);
    let selected = relative_path.to_string_lossy().replace('\\', "/");
    let (body, reference) = if is_skills_sh_source_body(body) {
        rewrite_skills_sh_selected_path(body, &selected, rewrite_mode, reference)
    } else if is_plain_github_url_body(body) {
        (
            append_github_url_selected_path(body, &selected, reference),
            None,
        )
    } else {
        (append_path_to_source_body(body, &selected), reference)
    };

    if let Some(reference) = reference {
        format!("{body}#{reference}")
    } else {
        body
    }
}

fn split_source_reference(source: &str) -> (&str, Option<&str>) {
    source
        .rsplit_once('#')
        .map_or((source, None), |(body, reference)| (body, Some(reference)))
}

fn is_skills_sh_source_body(body: &str) -> bool {
    skills_sh_prefix(body).is_some()
}

fn rewrite_skills_sh_selected_path<'a>(
    body: &str,
    selected: &str,
    rewrite_mode: SourceRewriteMode,
    reference: Option<&'a str>,
) -> (String, Option<&'a str>) {
    let Some(prefix) = skills_sh_prefix(body) else {
        return (append_path_to_source_body(body, selected), reference);
    };
    let rest = body.trim_start_matches(prefix).trim_matches('/');
    let parts = rest
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return (append_path_to_source_body(body, selected), reference);
    }

    let selected = selected.trim_matches('/');
    let selected_under_skills = selected == "skills" || selected.starts_with("skills/");
    if should_store_skills_sh_selection_as_github_tree(&parts, selected_under_skills, rewrite_mode)
    {
        return (
            github_tree_source_from_skills_sh_parts(parts[0], parts[1], selected, reference),
            None,
        );
    }

    let selected_path = selected.strip_prefix("skills/").unwrap_or(selected);
    let mut result = format!("{}{}/{}", prefix, parts[0], parts[1]);

    match rewrite_mode {
        SourceRewriteMode::Append => {
            if parts.len() > 2 {
                result.push('/');
                result.push_str(parts[2..].join("/").trim_matches('/'));
            }
            let selected_path = if parts.len() > 2 {
                selected
            } else {
                selected_path
            };
            if !selected_path.is_empty() {
                result.push('/');
                result.push_str(selected_path);
            }
        }
        SourceRewriteMode::ReplaceSkillsShPath => {
            if !selected_path.is_empty() {
                result.push('/');
                result.push_str(selected_path);
            }
        }
    }

    (result, reference)
}

fn should_store_skills_sh_selection_as_github_tree(
    parts: &[&str],
    selected_under_skills: bool,
    rewrite_mode: SourceRewriteMode,
) -> bool {
    !selected_under_skills
        && match rewrite_mode {
            SourceRewriteMode::ReplaceSkillsShPath => true,
            SourceRewriteMode::Append => parts.len() == 2,
        }
}

fn github_tree_source_from_skills_sh_parts(
    owner: &str,
    repo: &str,
    selected: &str,
    reference: Option<&str>,
) -> String {
    let reference = reference.unwrap_or("HEAD");
    format!("https://github.com/{owner}/{repo}/tree/{reference}/{selected}")
}

fn skills_sh_prefix(body: &str) -> Option<&'static str> {
    [
        "https://www.skills.sh/",
        "http://www.skills.sh/",
        "https://skills.sh/",
        "http://skills.sh/",
        "www.skills.sh/",
        "skills.sh/",
    ]
    .into_iter()
    .find(|prefix| body.starts_with(prefix))
}

fn is_plain_github_url_body(body: &str) -> bool {
    body.trim_end_matches('/')
        .starts_with("https://github.com/")
        && !body.contains("/tree/")
}

fn append_github_url_selected_path(body: &str, selected: &str, reference: Option<&str>) -> String {
    let trimmed = body.trim_end_matches('/');
    let selected = selected.trim_matches('/');
    if selected.is_empty() {
        return trimmed.to_owned();
    }
    let reference = reference.unwrap_or("HEAD");
    format!("{trimmed}/tree/{reference}/{selected}")
}

fn append_path_to_source_body(body: &str, selected: &str) -> String {
    let trimmed = body.trim_end_matches('/');
    let selected = selected.trim_matches('/');
    if selected.is_empty() {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/{selected}")
    }
}

pub fn infer_skill_name(source: &str) -> String {
    let without_ref = source.split('#').next().unwrap_or(source);
    let trimmed = without_ref.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .find(|part| !part.is_empty() && *part != "tree")
        .unwrap_or("skill")
        .trim_end_matches(".git")
        .to_owned()
}
