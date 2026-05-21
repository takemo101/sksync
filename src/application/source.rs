use crate::application::config::GitInstallSource;

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
    use super::{SkillsShProvider, SourceUrlTransformer};
    use std::path::Path;

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
