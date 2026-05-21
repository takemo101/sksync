use crate::application::config::{GitInstallSource, RegistryInstallSource};
use crate::application::ports::SkillInstallError;

pub trait RegistryProvider {
    fn supports(&self, registry: &str) -> bool;
    fn transform_url(&self, source: &str, reference: Option<&str>) -> Option<GitInstallSource>;
    fn resolve_git_source(
        &self,
        source: &RegistryInstallSource,
    ) -> Result<GitInstallSource, SkillInstallError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SkillsShProvider;

impl SkillsShProvider {
    pub fn repo_url(&self, source: &RegistryInstallSource) -> Option<String> {
        self.resolve_package(&source.registry, &source.package)
            .map(|package| package.repo_url)
    }

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

    fn resolve_package(&self, registry: &str, package: &str) -> Option<SkillsShPackage> {
        if !self.supports(registry) {
            return None;
        }

        self.package_from_path(package)
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
            parts[2..].join("/")
        } else {
            ".".to_owned()
        };

        Some(SkillsShPackage {
            repo_url: format!("https://github.com/{repo}.git"),
            path,
        })
    }
}

impl RegistryProvider for SkillsShProvider {
    fn supports(&self, registry: &str) -> bool {
        matches!(
            registry.trim().to_ascii_lowercase().as_str(),
            "skills.sh" | "skills-sh" | "www.skills.sh"
        )
    }

    fn transform_url(&self, source: &str, reference: Option<&str>) -> Option<GitInstallSource> {
        let package = self.resolve_url_source(source)?;
        Some(GitInstallSource {
            url: package.repo_url,
            reference: reference.map(str::to_owned),
            path: package.path.into(),
        })
    }

    fn resolve_git_source(
        &self,
        source: &RegistryInstallSource,
    ) -> Result<GitInstallSource, SkillInstallError> {
        let package = self
            .resolve_package(&source.registry, &source.package)
            .ok_or_else(|| {
                if self.supports(&source.registry) {
                    SkillInstallError::InvalidRegistryPackage {
                        registry: source.registry.clone(),
                        package: source.package.clone(),
                        message: "skills.sh package must be owner/repo[/path/to/skill]".to_owned(),
                    }
                } else {
                    SkillInstallError::UnsupportedRegistry {
                        registry: source.registry.clone(),
                        package: source.package.clone(),
                    }
                }
            })?;

        Ok(GitInstallSource {
            url: package.repo_url,
            reference: source.reference.clone(),
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
pub struct RegistryProviders {
    skills_sh: SkillsShProvider,
}

impl RegistryProviders {
    pub fn transform_url(&self, source: &str, reference: Option<&str>) -> Option<GitInstallSource> {
        self.skills_sh.transform_url(source, reference)
    }

    pub fn resolve_git_source(
        &self,
        source: &RegistryInstallSource,
    ) -> Result<GitInstallSource, SkillInstallError> {
        if self.skills_sh.supports(&source.registry) {
            return self.skills_sh.resolve_git_source(source);
        }

        Err(SkillInstallError::UnsupportedRegistry {
            registry: source.registry.clone(),
            package: source.package.clone(),
        })
    }

    pub fn git_repo_url(&self, source: &RegistryInstallSource) -> Option<String> {
        if self.skills_sh.supports(&source.registry) {
            return self.skills_sh.repo_url(source);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{RegistryProvider, RegistryProviders, SkillsShProvider};
    use crate::application::config::RegistryInstallSource;
    use crate::application::ports::SkillInstallError;
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
        assert_eq!(git.path, Path::new("find-skills"));
    }

    #[test]
    fn skills_sh_provider_maps_shorthand_url_to_github_git_source() {
        let git = SkillsShProvider
            .transform_url("skills.sh/owner/repo/path/to/skill", None)
            .expect("skills.sh shorthand maps to git");

        assert_eq!(git.url, "https://github.com/owner/repo.git");
        assert_eq!(git.reference, None);
        assert_eq!(git.path, Path::new("path/to/skill"));
    }

    #[test]
    fn skills_sh_provider_maps_registry_source_to_github_git_source() {
        let git = SkillsShProvider
            .resolve_git_source(&RegistryInstallSource {
                registry: "skills.sh".to_owned(),
                package: "owner/repo/path/to/skill".to_owned(),
                reference: Some("v1".to_owned()),
            })
            .expect("skills.sh source maps to git");

        assert_eq!(git.url, "https://github.com/owner/repo.git");
        assert_eq!(git.reference.as_deref(), Some("v1"));
        assert_eq!(git.path, Path::new("path/to/skill"));
    }

    #[test]
    fn skills_sh_provider_accepts_repo_only_package() {
        let git = SkillsShProvider
            .resolve_git_source(&RegistryInstallSource {
                registry: "skills-sh".to_owned(),
                package: "owner/repo".to_owned(),
                reference: None,
            })
            .expect("skills-sh alias maps to git");

        assert_eq!(git.url, "https://github.com/owner/repo.git");
        assert_eq!(git.reference, None);
        assert_eq!(git.path, Path::new("."));
    }

    #[test]
    fn registry_providers_keep_unknown_registries_unsupported() {
        let error = RegistryProviders::default()
            .resolve_git_source(&RegistryInstallSource {
                registry: "example.com".to_owned(),
                package: "owner/repo/skill".to_owned(),
                reference: None,
            })
            .expect_err("other registries remain unsupported");

        assert!(matches!(
            error,
            SkillInstallError::UnsupportedRegistry { .. }
        ));
    }

    #[test]
    fn skills_sh_provider_rejects_package_without_repo() {
        let error = SkillsShProvider
            .resolve_git_source(&RegistryInstallSource {
                registry: "skills.sh".to_owned(),
                package: "owner-only".to_owned(),
                reference: None,
            })
            .expect_err("owner-only package is invalid");

        assert!(matches!(
            error,
            SkillInstallError::InvalidRegistryPackage { .. }
        ));
    }

    #[test]
    fn registry_providers_return_git_repo_url_for_skills_sh() {
        let repo = RegistryProviders::default().git_repo_url(&RegistryInstallSource {
            registry: "skills.sh".to_owned(),
            package: "owner/repo/path/to/skill".to_owned(),
            reference: None,
        });

        assert_eq!(repo.as_deref(), Some("https://github.com/owner/repo.git"));
    }
}
