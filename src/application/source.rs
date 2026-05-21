use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::application::config::{GitInstallSource, InstallSource};

pub trait SourceUrlTransformer {
    fn transform_url(&self, source: &str, reference: Option<&str>) -> Option<GitInstallSource>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SkillsShProvider;

impl SkillsShProvider {
    fn resolve_url_source(&self, source: &str) -> Option<SkillsShPackage> {
        let body = source
            .strip_prefix("https://www.skills.sh/")
            .or_else(|| source.strip_prefix("http://www.skills.sh/"))
            .or_else(|| source.strip_prefix("https://skills.sh/"))
            .or_else(|| source.strip_prefix("http://skills.sh/"))
            .or_else(|| source.strip_prefix("www.skills.sh/"))
            .or_else(|| source.strip_prefix("skills.sh/"))?;

        self.package_from_path(body)
    }

    fn package_from_path(&self, value: &str) -> Option<SkillsShPackage> {
        let parts = value
            .trim_matches('/')
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.len() < 2 {
            return None;
        }

        let repo = format!("{}/{}", parts[0], parts[1]);
        let path = if parts.len() > 2 {
            format!("skills/{}", parts[2..].join("/"))
        } else {
            ".".to_owned()
        };

        Some(SkillsShPackage {
            repo_url: format!("https://github.com/{repo}.git"),
            path,
        })
    }
}

impl SourceUrlTransformer for SkillsShProvider {
    fn transform_url(&self, source: &str, reference: Option<&str>) -> Option<GitInstallSource> {
        let package = self.resolve_url_source(source)?;
        Some(GitInstallSource {
            url: package.repo_url,
            reference: reference.map(str::to_owned),
            path: package.path.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillsShPackage {
    repo_url: String,
    path: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SourceParseError {
    #[error("unsupported source '{value}'")]
    Unsupported { value: String },
    #[error("registry sources are not supported; use a provider URL such as https://www.skills.sh/owner/repo/skill-name")]
    LegacyRegistry,
    #[error("git source path '{path}' must be relative and must not contain '..'")]
    InvalidGitSubpath { path: String },
}

pub fn parse_install_source_string(value: &str) -> Result<InstallSource, SourceParseError> {
    if value.starts_with("./") || value.starts_with("../") || value.starts_with('/') {
        return Ok(InstallSource::Local(PathBuf::from(value)));
    }

    let (body, reference) = split_ref(value);
    if let Some(mut transformed) = SourceUrlTransformers::default().transform_url(body, reference) {
        transformed.path = validate_git_subpath(transformed.path)?;
        return Ok(InstallSource::Git(transformed));
    }

    if let Some(mut parsed) = parse_github_tree_url(value) {
        parsed.path = validate_git_subpath(parsed.path)?;
        return Ok(InstallSource::Git(parsed));
    }

    if body.starts_with("registry:") {
        return Err(SourceParseError::LegacyRegistry);
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
        let path = validate_git_subpath(PathBuf::from(path))?;
        return Ok(InstallSource::Git(GitInstallSource {
            url: git_url_from_repo(&repo),
            reference: reference.map(str::to_owned),
            path,
        }));
    }

    Err(SourceParseError::Unsupported {
        value: value.to_owned(),
    })
}

pub fn validate_git_subpath(path: PathBuf) -> Result<PathBuf, SourceParseError> {
    if !is_safe_git_subpath(&path) {
        return Err(SourceParseError::InvalidGitSubpath {
            path: path.display().to_string(),
        });
    }
    Ok(path)
}

fn is_safe_git_subpath(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::CurDir | Component::Normal(_)))
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

pub fn git_url_from_repo(repo_or_url: &str) -> String {
    if repo_or_url.contains("://") || repo_or_url.ends_with(".git") {
        repo_or_url.to_owned()
    } else {
        format!("https://github.com/{repo_or_url}.git")
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SourceUrlTransformers {
    skills_sh: SkillsShProvider,
}

impl SourceUrlTransformers {
    pub fn transform_url(&self, source: &str, reference: Option<&str>) -> Option<GitInstallSource> {
        self.skills_sh.transform_url(source, reference)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_install_source_string, SkillsShProvider, SourceParseError, SourceUrlTransformer,
    };
    use crate::application::config::InstallSource;
    use std::path::Path;

    #[test]
    fn github_shorthand_source_parses_as_git_source() {
        let source =
            parse_install_source_string("owner/repo/skills/review#main").expect("source parses");
        let InstallSource::Git(git) = source else {
            panic!("expected git source");
        };

        assert_eq!(git.url, "https://github.com/owner/repo.git");
        assert_eq!(git.reference.as_deref(), Some("main"));
        assert_eq!(git.path, Path::new("skills/review"));
    }

    #[test]
    fn github_tree_url_source_parses_as_git_source() {
        let source =
            parse_install_source_string("https://github.com/owner/repo/tree/main/skills/review")
                .expect("source parses");
        let InstallSource::Git(git) = source else {
            panic!("expected git source");
        };

        assert_eq!(git.url, "https://github.com/owner/repo.git");
        assert_eq!(git.reference.as_deref(), Some("main"));
        assert_eq!(git.path, Path::new("skills/review"));
    }

    #[test]
    fn source_parser_rejects_parent_directory_git_subpath() {
        assert!(matches!(
            parse_install_source_string("owner/repo/../review#main"),
            Err(SourceParseError::InvalidGitSubpath { .. })
        ));
    }

    #[test]
    fn source_parser_rejects_legacy_registry_source() {
        assert_eq!(
            parse_install_source_string("registry:skills.sh/owner/repo/review"),
            Err(SourceParseError::LegacyRegistry)
        );
    }

    #[test]
    fn skills_sh_provider_maps_url_to_github_git_source() {
        let git = SkillsShProvider
            .transform_url(
                "https://www.skills.sh/vercel-labs/skills/find-skills",
                Some("main"),
            )
            .expect("skills.sh url maps to git");

        assert_eq!(git.url, "https://github.com/vercel-labs/skills.git");
        assert_eq!(git.reference.as_deref(), Some("main"));
        assert_eq!(git.path, Path::new("skills/find-skills"));
    }

    #[test]
    fn skills_sh_provider_maps_shorthand_url_to_github_git_source() {
        let git = SkillsShProvider
            .transform_url("skills.sh/owner/repo/my-skill", None)
            .expect("skills.sh shorthand maps to git");

        assert_eq!(git.url, "https://github.com/owner/repo.git");
        assert_eq!(git.reference, None);
        assert_eq!(git.path, Path::new("skills/my-skill"));
    }
}
